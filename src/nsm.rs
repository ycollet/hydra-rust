//! Shared NSM (New/Non Session Manager) OSC client for the two standalone apps.

use std::io;
use std::net::{ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rosc::{OscMessage, OscPacket, OscType};
use serde::{Serialize, de::DeserializeOwned};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn load<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err),
    }
}

/// Replace a snapshot only after its complete contents have reached disk.
pub fn save(path: &Path, state: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Missing session directory"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub struct NsmClient {
    socket: UdpSocket,
    pub project_dir: PathBuf,
    pub display_name: String,
    pub client_id: String,
}

impl NsmClient {
    /// Invalid or unreachable managers leave the application in normal standalone mode.
    pub fn from_env(name: &str) -> Option<Self> {
        let url = std::env::var_os("NSM_URL")?;
        let executable = std::env::args().next().unwrap_or_else(|| name.to_owned());
        match url
            .to_str()
            .ok_or_else(|| invalid("NSM_URL is not UTF-8"))
            .and_then(|url| Self::connect(url, name, &executable))
        {
            Ok(client) => Some(client),
            Err(err) => {
                eprintln!("NSM unavailable: {err}; continuing without session management");
                None
            }
        }
    }

    fn connect(url: &str, name: &str, executable: &str) -> io::Result<Self> {
        let authority = url
            .strip_prefix("osc.udp://")
            .ok_or_else(|| invalid("NSM_URL must use osc.udp://"))?
            .trim_end_matches('/');
        let address = authority
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| invalid("NSM_URL has no address"))?;
        let socket = UdpSocket::bind(if address.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })?;
        socket.connect(address)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut client = Self {
            socket,
            project_dir: PathBuf::new(),
            display_name: String::new(),
            client_id: String::new(),
        };
        client.send(
            "/nsm/server/announce",
            vec![
                name.into(),
                "".into(),
                executable.into(),
                OscType::Int(1),
                OscType::Int(1),
                OscType::Int(std::process::id() as i32),
            ],
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut announced = false;
        while Instant::now() < deadline {
            let message = match client.receive() {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(err) => return Err(err),
            };
            match (message.addr.as_str(), message.args.as_slice()) {
                (
                    "/reply",
                    [
                        OscType::String(path),
                        OscType::String(_),
                        OscType::String(_),
                        OscType::String(_),
                    ],
                ) if path == "/nsm/server/announce" => announced = true,
                ("/error", _) => {
                    return Err(invalid(format!(
                        "NSM rejected announce: {:?}",
                        message.args
                    )));
                }
                (
                    "/nsm/client/open",
                    [
                        OscType::String(path),
                        OscType::String(display),
                        OscType::String(id),
                    ],
                ) if announced => {
                    if !Path::new(path).is_absolute() {
                        client.reply(
                            "/nsm/client/open",
                            Err(invalid("NSM project path must be absolute")),
                        )?;
                        return Err(invalid("NSM project path must be absolute"));
                    }
                    client.project_dir = PathBuf::from(path);
                    client.display_name = display.clone();
                    client.client_id = id.clone();
                    client.socket.set_nonblocking(true)?;
                    return Ok(client);
                }
                _ => {}
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "NSM announce/open timed out",
        ))
    }

    fn send(&self, address: &str, args: Vec<OscType>) -> io::Result<()> {
        let bytes = rosc::encoder::encode(&OscPacket::Message(OscMessage {
            addr: address.to_owned(),
            args,
        }))
        .map_err(io::Error::other)?;
        self.socket.send(&bytes)?;
        Ok(())
    }

    fn receive(&self) -> io::Result<Option<OscMessage>> {
        let mut bytes = [0; 65536];
        let size = self.socket.recv(&mut bytes)?;
        match rosc::decoder::decode_udp(&bytes[..size])
            .map_err(io::Error::other)?
            .1
        {
            OscPacket::Message(message) => Ok(Some(message)),
            OscPacket::Bundle(_) => Err(invalid("NSM control messages must not be bundled")),
        }
    }

    pub fn reply(&self, path: &str, result: io::Result<()>) -> io::Result<()> {
        match result {
            Ok(()) => self.send("/reply", vec![path.into(), "OK".into()]),
            Err(err) => {
                log::error!("NSM {path}: {err}");
                self.send(
                    "/error",
                    vec![path.into(), OscType::Int(-1), err.to_string().into()],
                )
            }
        }
    }

    /// Called off the audio thread. Success is acknowledged only after the snapshot is written.
    pub fn poll_save(&self, mut save: impl FnMut() -> io::Result<()>) -> io::Result<()> {
        // Bound work per GUI frame even if the manager sends a burst of messages.
        for _ in 0..32 {
            let message = match self.receive() {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => return Err(err),
            };
            match message.addr.as_str() {
                "/nsm/client/save" if message.args.is_empty() => {
                    self.reply(&message.addr, save())?
                }
                "/nsm/client/session_is_loaded" => {}
                "/nsm/client/open" => self.reply(
                    &message.addr,
                    Err(invalid("Session switching is not supported")),
                )?,
                path if path.starts_with("/nsm/client/") => {
                    self.reply(path, Err(invalid("Unsupported NSM command")))?
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(socket: &UdpSocket, addr: std::net::SocketAddr, path: &str, args: Vec<OscType>) {
        socket
            .send_to(
                &rosc::encoder::encode(&OscPacket::Message(OscMessage {
                    addr: path.into(),
                    args,
                }))
                .unwrap(),
                addr,
            )
            .unwrap();
    }

    fn receive(socket: &UdpSocket) -> (OscMessage, std::net::SocketAddr) {
        let mut buffer = [0; 4096];
        let (size, addr) = socket.recv_from(&mut buffer).unwrap();
        let OscPacket::Message(message) = rosc::decoder::decode_udp(&buffer[..size]).unwrap().1
        else {
            panic!("Expected OSC message")
        };
        (message, addr)
    }

    #[test]
    fn manager_handshake_save_and_errors_use_the_announce_socket() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let url = format!("osc.udp://{}/", server.local_addr().unwrap());
        let manager = std::thread::spawn(move || {
            let (announce, addr) = receive(&server);
            assert_eq!(announce.addr, "/nsm/server/announce");
            assert_eq!(
                &announce.args[..3],
                &["Hydra".into(), "".into(), "hydra-test".into()]
            );
            send(
                &server,
                addr,
                "/reply",
                vec![
                    "/nsm/server/announce".into(),
                    "Welcome".into(),
                    "NSM".into(),
                    ":server-control:".into(),
                ],
            );
            send(
                &server,
                addr,
                "/nsm/client/open",
                vec![
                    "/tmp/hydra-test.ID".into(),
                    "Test".into(),
                    "Hydra.ID".into(),
                ],
            );
            let (opened, source) = receive(&server);
            assert_eq!(source, addr);
            assert_eq!(opened.addr, "/reply");
            send(&server, addr, "/nsm/client/save", vec![]);
            let (saved, source) = receive(&server);
            assert_eq!(source, addr);
            assert_eq!(saved.addr, "/reply");
            send(&server, addr, "/nsm/client/save", vec![]);
            let (error, source) = receive(&server);
            assert_eq!(source, addr);
            assert_eq!(error.addr, "/error");
            assert_eq!(error.args[1], OscType::Int(-1));
        });
        let client = NsmClient::connect(&url, "Hydra", "hydra-test").unwrap();
        assert_eq!(client.project_dir, Path::new("/tmp/hydra-test.ID"));
        assert_eq!(client.client_id, "Hydra.ID");
        client.reply("/nsm/client/open", Ok(())).unwrap();
        let mut saves = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while saves < 2 && Instant::now() < deadline {
            client
                .poll_save(|| {
                    saves += 1;
                    if saves == 1 {
                        Ok(())
                    } else {
                        Err(io::Error::other("Disk full"))
                    }
                })
                .unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(saves, 2);
        manager.join().unwrap();
    }

    #[test]
    fn invalid_urls_are_rejected() {
        for url in [
            "http://localhost:1234",
            "osc.tcp://localhost:1234",
            "osc.udp://",
        ] {
            assert!(NsmClient::connect(url, "Hydra", "hydra").is_err());
        }
    }

    #[test]
    fn snapshots_round_trip_on_disk_and_corrupt_files_are_not_silently_reset() {
        let directory = std::env::temp_dir().join(format!(
            "hydra-nsm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let path = directory.join("session.json");
        assert!(load::<Vec<String>>(&path).unwrap().is_empty());
        let state = vec!["osc(10).out()".to_owned(), "noise(2).out()".to_owned()];
        save(&path, &state).unwrap();
        assert_eq!(load::<Vec<String>>(&path).unwrap(), state);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "broken").unwrap();
        assert!(load::<Vec<String>>(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
