use baseview::{PhySize, Size, WindowInfo};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

/// NIH-plug does not forward host-initiated resizes to the editor. Watch the
/// embedding window without changing the host's own event subscriptions.
pub(crate) struct X11ParentWindow {
    connection: RustConnection,
    parent: u32,
    physical_size: (u32, u32),
    fullscreen_window: Option<(u32, u32)>,
}

impl X11ParentWindow {
    pub(crate) fn new(parent: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let (connection, _) = x11rb::connect(None)?;
        connection
            .change_window_attributes(
                parent,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )?
            .check()?;
        let geometry = connection.get_geometry(parent)?.reply()?;
        Ok(Self {
            connection,
            parent,
            physical_size: (geometry.width as u32, geometry.height as u32),
            fullscreen_window: None,
        })
    }

    fn fullscreen_target(&self) -> Result<(u32, u32), Box<dyn std::error::Error>> {
        let wm_state = self
            .connection
            .intern_atom(false, b"WM_STATE")?
            .reply()?
            .atom;
        let mut window = self.parent;
        loop {
            let tree = self.connection.query_tree(window)?.reply()?;
            if window == tree.root {
                return Err(std::io::Error::other("No host top-level window found").into());
            }
            // A reparenting WM places a frame above the client window. Target
            // the ICCCM client, not that frame or the plugin's embedded child.
            let state = self
                .connection
                .get_property(false, window, wm_state, AtomEnum::ANY, 0, 2)?
                .reply()?;
            if state.type_ != u32::from(AtomEnum::NONE) || tree.parent == tree.root {
                return Ok((tree.root, window));
            }
            window = tree.parent;
        }
    }

    fn send_fullscreen(
        &self,
        root: u32,
        window: u32,
        fullscreen: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let state = self
            .connection
            .intern_atom(false, b"_NET_WM_STATE")?
            .reply()?
            .atom;
        let fullscreen_atom = self
            .connection
            .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")?
            .reply()?
            .atom;
        self.connection
            .send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ClientMessageEvent::new(
                    32,
                    window,
                    state,
                    [u32::from(fullscreen), fullscreen_atom, 0, 1, 0],
                ),
            )?
            .check()?;
        Ok(())
    }

    pub(crate) fn set_fullscreen(
        &mut self,
        fullscreen: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (root, window) = match self.fullscreen_window {
            Some(target) => target,
            None => self.fullscreen_target()?,
        };
        let supported_atom = self
            .connection
            .intern_atom(false, b"_NET_SUPPORTED")?
            .reply()?
            .atom;
        let fullscreen_atom = self
            .connection
            .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")?
            .reply()?
            .atom;
        let supported = self
            .connection
            .get_property(false, root, supported_atom, AtomEnum::ATOM, 0, u32::MAX)?
            .reply()?;
        if !supported
            .value32()
            .is_some_and(|mut atoms| atoms.any(|atom| atom == fullscreen_atom))
        {
            return Err(std::io::Error::other(
                "The X11 window manager does not support fullscreen",
            )
            .into());
        }
        self.send_fullscreen(root, window, fullscreen)?;
        self.fullscreen_window = fullscreen.then_some((root, window));
        Ok(())
    }

    pub(crate) fn poll_resize(&mut self, scale: f64) -> Result<Option<Size>, ReplyError> {
        let mut size = self.physical_size;
        while let Some(event) = self.connection.poll_for_event()? {
            match event {
                Event::ConfigureNotify(event) if event.window == self.parent => {
                    if event.width > 0 && event.height > 0 {
                        size = (event.width as u32, event.height as u32);
                    }
                }
                Event::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        if size == self.physical_size {
            return Ok(None);
        }
        self.physical_size = size;
        Ok(Some(
            WindowInfo::from_physical_size(PhySize::new(size.0, size.1), scale).logical_size(),
        ))
    }
}

impl Drop for X11ParentWindow {
    fn drop(&mut self) {
        if let Some((root, window)) = self.fullscreen_window {
            if let Err(err) = self.send_fullscreen(root, window, false) {
                log::warn!("hydra-plugin: cannot leave fullscreen while closing the editor: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{ConfigureWindowAux, CreateWindowAux, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::COPY_DEPTH_FROM_PARENT;

    fn parent_window() -> (RustConnection, u32, X11ParentWindow) {
        let (host, screen) = x11rb::connect(None).expect("connect to test X11 server");
        let parent = host.generate_id().unwrap();
        host.create_window(
            COPY_DEPTH_FROM_PARENT,
            parent,
            host.setup().roots[screen].root,
            0,
            0,
            1024,
            576,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .unwrap()
        .check()
        .unwrap();
        let watcher = X11ParentWindow::new(parent).unwrap();
        (host, parent, watcher)
    }

    fn sync(watcher: &X11ParentWindow) {
        watcher
            .connection
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
    }

    #[test]
    #[ignore = "requires an X11 server (e.g. Xvfb)"]
    fn follows_maximize_and_restore_at_both_pixel_scales() {
        for scale in [1.0, 2.0] {
            let (host, parent, mut watcher) = parent_window();
            assert!(watcher.poll_resize(scale).unwrap().is_none());
            for (width, height) in [(1920, 1080), (1024, 576)] {
                host.configure_window(
                    parent,
                    &ConfigureWindowAux::new().width(width).height(height),
                )
                .unwrap()
                .check()
                .unwrap();
                sync(&watcher);
                let size = watcher.poll_resize(scale).unwrap().unwrap();
                assert_eq!(size.width, width as f64 / scale);
                assert_eq!(size.height, height as f64 / scale);
                assert!(watcher.poll_resize(scale).unwrap().is_none());

                // The independent watcher must not steal the host's events.
                assert!(matches!(
                    host.poll_for_event().unwrap(),
                    Some(Event::ConfigureNotify(_))
                ));
            }
        }
    }

    #[test]
    #[ignore = "requires an X11 server (e.g. Xvfb)"]
    fn ignores_moves_and_coalesces_resize_events() {
        let (host, parent, mut watcher) = parent_window();
        host.configure_window(parent, &ConfigureWindowAux::new().x(100).y(100))
            .unwrap()
            .check()
            .unwrap();
        sync(&watcher);
        assert!(watcher.poll_resize(1.0).unwrap().is_none());

        for (width, height) in [(1280, 720), (1920, 1080), (1600, 900)] {
            host.configure_window(
                parent,
                &ConfigureWindowAux::new().width(width).height(height),
            )
            .unwrap()
            .check()
            .unwrap();
        }
        sync(&watcher);
        let size = watcher.poll_resize(1.0).unwrap().unwrap();
        assert_eq!((size.width, size.height), (1600.0, 900.0));
    }

    #[test]
    #[ignore = "requires an isolated X11 server (e.g. Xvfb)"]
    fn fullscreen_targets_the_host_client_and_removes_only_fullscreen_on_exit_or_close() {
        let (host, client, watcher) = parent_window();
        let root = host.query_tree(client).unwrap().reply().unwrap().root;
        let frame = host.generate_id().unwrap();
        let embedded = host.generate_id().unwrap();
        for (window, parent) in [(frame, root), (embedded, client)] {
            host.create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                parent,
                0,
                0,
                1024,
                576,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        }
        host.reparent_window(client, frame, 0, 0)
            .unwrap()
            .check()
            .unwrap();

        let atom = |name| host.intern_atom(false, name).unwrap().reply().unwrap().atom;
        let wm_state = atom(b"WM_STATE");
        let state = atom(b"_NET_WM_STATE");
        let fullscreen = atom(b"_NET_WM_STATE_FULLSCREEN");
        let supported = atom(b"_NET_SUPPORTED");
        host.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            client,
            wm_state,
            wm_state,
            &[1, 0],
        )
        .unwrap()
        .check()
        .unwrap();
        host.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            root,
            supported,
            AtomEnum::ATOM,
            &[fullscreen],
        )
        .unwrap()
        .check()
        .unwrap();
        host.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .unwrap()
        .check()
        .unwrap();
        drop(watcher);
        let mut watcher = X11ParentWindow::new(embedded).unwrap();
        let request = |enabled| {
            host.get_input_focus().unwrap().reply().unwrap();
            let mut requests = Vec::new();
            while let Some(event) = host.poll_for_event().unwrap() {
                if let Event::ClientMessage(event) = event {
                    if event.type_ == state {
                        assert_eq!(event.window, client);
                        assert_eq!(event.format, 32);
                        requests.push(event.data.as_data32());
                    }
                }
            }
            assert_eq!(requests, vec![[u32::from(enabled), fullscreen, 0, 1, 0]]);
        };
        watcher.set_fullscreen(true).unwrap();
        request(true);
        watcher.set_fullscreen(false).unwrap();
        request(false);
        watcher.set_fullscreen(true).unwrap();
        request(true);
        drop(watcher);
        request(false);

        host.delete_property(root, supported)
            .unwrap()
            .check()
            .unwrap();
        let mut watcher = X11ParentWindow::new(embedded).unwrap();
        assert!(watcher.set_fullscreen(true).is_err());
        assert!(watcher.fullscreen_window.is_none());
    }
}
