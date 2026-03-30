pub const NUM_SOURCES: usize = 4;

pub struct SourceFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "webcam")]
mod imp {
    use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
    use std::sync::Once;
    use std::thread;
    use std::time::Duration;

    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    use super::SourceFrame;

    pub const NUM_SOURCES: usize = super::NUM_SOURCES;

    static INIT: Once = Once::new();

    fn request_permission() {
        INIT.call_once(|| {
            nokhwa::nokhwa_initialize(|granted| {
                if granted {
                    log::info!("camera permission granted");
                } else {
                    log::warn!("camera permission denied");
                }
            });
        });
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum CameraStatus {
        Idle,
        Opening { camera_index: u32 },
        Active { camera_index: u32, camera_name: String, width: u32, height: u32 },
        Error { camera_index: u32, message: String },
    }

    pub struct CameraInfo {
        pub index: u32,
        pub name: String,
    }

    enum SlotMessage {
        Frame(SourceFrame),
        Status(CameraStatus),
    }

    enum CameraCommand {
        Open(u32),
        Close,
        Shutdown,
    }

    struct CameraSlot {
        command_tx: mpsc::Sender<CameraCommand>,
        message_rx: Receiver<SlotMessage>,
        status: CameraStatus,
        current_camera: Option<u32>,
    }

    impl CameraSlot {
        fn new(slot: usize) -> Self {
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let (msg_tx, msg_rx) = mpsc::sync_channel(2);

            thread::Builder::new()
                .name(format!("hydra-cam-{slot}"))
                .spawn(move || slot_thread(slot, cmd_rx, msg_tx))
                .expect("spawn camera slot thread");

            Self {
                command_tx: cmd_tx,
                message_rx: msg_rx,
                status: CameraStatus::Idle,
                current_camera: None,
            }
        }
    }

    impl Drop for CameraSlot {
        fn drop(&mut self) {
            let _ = self.command_tx.send(CameraCommand::Shutdown);
        }
    }

    #[derive(Default)]
    pub struct SourceManager {
        slots: [Option<CameraSlot>; NUM_SOURCES],
        cached_cameras: Vec<CameraInfo>,
    }

    impl SourceManager {
        pub fn new() -> Self {
            request_permission();
            let mut mgr = Self::default();
            mgr.refresh_cameras();
            mgr
        }

        pub fn init_cam(&mut self, slot: usize, camera_index: u32) {
            if slot >= NUM_SOURCES {
                return;
            }

            let camera_slot = self.slots[slot].get_or_insert_with(|| CameraSlot::new(slot));

            if camera_slot.current_camera == Some(camera_index) {
                return;
            }

            if camera_slot.command_tx.send(CameraCommand::Open(camera_index)).is_ok() {
                camera_slot.current_camera = Some(camera_index);
            } else {
                self.slots[slot] = None;
                self.slots[slot] = Some(CameraSlot::new(slot));
                let s = self.slots[slot].as_mut().unwrap();
                let _ = s.command_tx.send(CameraCommand::Open(camera_index));
                s.current_camera = Some(camera_index);
            }
        }

        pub fn close_cam(&mut self, slot: usize) {
            if slot >= NUM_SOURCES {
                return;
            }
            if let Some(s) = &mut self.slots[slot] {
                let _ = s.command_tx.send(CameraCommand::Close);
                s.current_camera = None;
            }
        }

        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            let s = self.slots[slot].as_mut()?;
            let mut latest_frame = None;

            while let Ok(msg) = s.message_rx.try_recv() {
                match msg {
                    SlotMessage::Frame(f) => latest_frame = Some(f),
                    SlotMessage::Status(status) => s.status = status,
                }
            }

            latest_frame
        }

        pub fn status(&self, slot: usize) -> &CameraStatus {
            static IDLE: CameraStatus = CameraStatus::Idle;
            if slot >= NUM_SOURCES {
                return &IDLE;
            }
            self.slots[slot].as_ref().map_or(&IDLE, |s| &s.status)
        }

        pub fn refresh_cameras(&mut self) {
            self.cached_cameras = nokhwa::query(ApiBackend::Auto)
                .unwrap_or_default()
                .into_iter()
                .map(|info| {
                    let index = match info.index() {
                        CameraIndex::Index(i) => *i,
                        CameraIndex::String(s) => s.parse().unwrap_or(0),
                    };
                    CameraInfo { index, name: info.human_name().to_string() }
                })
                .collect();
        }

        pub fn cameras(&self) -> &[CameraInfo] {
            &self.cached_cameras
        }

        pub fn stop(&mut self, slot: usize) {
            if slot < NUM_SOURCES {
                self.slots[slot] = None;
            }
        }

        pub fn stop_all(&mut self) {
            for i in 0..NUM_SOURCES {
                self.stop(i);
            }
        }
    }

    fn slot_thread(
        slot: usize,
        command_rx: mpsc::Receiver<CameraCommand>,
        message_tx: SyncSender<SlotMessage>,
    ) {
        let mut camera: Option<Camera> = None;
        let mut active_index: u32 = 0;

        loop {
            let cmd = if camera.is_some() {
                command_rx.try_recv().ok()
            } else {
                match command_rx.recv() {
                    Ok(cmd) => Some(cmd),
                    Err(_) => break,
                }
            };

            match cmd {
                Some(CameraCommand::Open(idx)) => {
                    active_index = idx;
                    if let Some(mut cam) = camera.take() {
                        let _ = cam.stop_stream();
                        drop(cam);
                        thread::sleep(Duration::from_millis(50));
                    }
                    let _ = message_tx.try_send(SlotMessage::Status(CameraStatus::Opening {
                        camera_index: idx,
                    }));
                    camera = open_camera(slot, idx, &message_tx);
                }
                Some(CameraCommand::Close) => {
                    if let Some(mut cam) = camera.take() {
                        let _ = cam.stop_stream();
                    }
                    let _ = message_tx.try_send(SlotMessage::Status(CameraStatus::Idle));
                }
                Some(CameraCommand::Shutdown) => {
                    if let Some(mut cam) = camera.take() {
                        let _ = cam.stop_stream();
                    }
                    break;
                }
                None => {}
            }

            if let Some(ref mut cam) = camera {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cam.frame()));

                match result {
                    Ok(Ok(buf)) => match buf.decode_image::<RgbFormat>() {
                        Ok(img) => {
                            let frame = SourceFrame {
                                width: img.width(),
                                height: img.height(),
                                pixels: img.into_raw(),
                            };
                            match message_tx.try_send(SlotMessage::Frame(frame)) {
                                Ok(()) | Err(TrySendError::Full(_)) => {}
                                Err(TrySendError::Disconnected(_)) => break,
                            }
                        }
                        Err(e) => {
                            log::warn!("slot {slot} decode error: {e}");
                        }
                    },
                    Ok(Err(e)) => {
                        let msg = e.to_string();
                        log::warn!("slot {slot} frame error: {msg}");
                        let _ =
                            message_tx.try_send(SlotMessage::Status(CameraStatus::Error {
                                camera_index: active_index,
                                message: msg,
                            }));
                        camera = None;
                    }
                    Err(_) => {
                        log::warn!("slot {slot} camera crashed (caught panic)");
                        let _ =
                            message_tx.try_send(SlotMessage::Status(CameraStatus::Error {
                                camera_index: active_index,
                                message: "camera crashed".into(),
                            }));
                        camera = None;
                    }
                }

                thread::sleep(Duration::from_millis(16));
            }
        }

        if let Some(mut cam) = camera.take() {
            let _ = cam.stop_stream();
        }
        log::info!("slot {slot} thread exited");
    }

    fn try_open(
        index: u32,
        format_type: RequestedFormatType,
    ) -> Result<Camera, nokhwa::NokhwaError> {
        Camera::new(
            CameraIndex::Index(index),
            RequestedFormat::new::<RgbFormat>(format_type),
        )
    }

    fn open_camera(
        slot: usize,
        index: u32,
        tx: &SyncSender<SlotMessage>,
    ) -> Option<Camera> {
        let mut camera = match try_open(index, RequestedFormatType::None)
            .or_else(|_| try_open(index, RequestedFormatType::AbsoluteHighestFrameRate))
            .or_else(|_| try_open(index, RequestedFormatType::AbsoluteHighestResolution))
        {
            Ok(c) => c,
            Err(e) => {
                log::warn!("slot {slot}: failed to open camera {index}: {e}");
                let _ = tx.try_send(SlotMessage::Status(CameraStatus::Error {
                    camera_index: index,
                    message: format!("failed to open: {e}"),
                }));
                return None;
            }
        };

        if let Err(e) = camera.open_stream() {
            log::warn!("slot {slot}: failed to start camera {index}: {e}");
            let _ = tx.try_send(SlotMessage::Status(CameraStatus::Error {
                camera_index: index,
                message: format!("failed to start stream: {e}"),
            }));
            return None;
        }

        let name = camera.info().human_name().to_string();
        let res = camera.resolution();
        log::info!(
            "slot {slot}: camera {index} ({name}) started at {}x{}",
            res.width(),
            res.height()
        );

        let _ = tx.try_send(SlotMessage::Status(CameraStatus::Active {
            camera_index: index,
            camera_name: name,
            width: res.width(),
            height: res.height(),
        }));

        Some(camera)
    }
}

#[cfg(feature = "webcam")]
pub use imp::*;
