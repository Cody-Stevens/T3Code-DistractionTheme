//! Python-compatible client and planned server, porting `glowtube.ctl/command/serve`.

use crate::{cli::CtlCommand, player::Player};
use dt_core::{Error, Result, constants::CONTROL_TIMEOUT};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// JSON snapshot returned by every successful `glowtube.serve` request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ControlStatus {
    /// Current title, null without a track.
    pub track: Option<String>,
    /// Whether playback is enabled.
    pub playing: bool,
    /// Transient card status text.
    pub status: String,
    /// Audible media seconds rounded to one decimal place.
    pub time: f64,
    /// Ordered queue titles (including the current and preceding tracks).
    pub queue: Vec<String>,
    /// Current queue index.
    pub index: usize,
    /// Gain in 0..=1.5.
    pub volume: f64,
    /// Intensity in 0..=11.
    pub chaos: f64,
    /// Display offset in seconds, rounded to three decimals (may be negative).
    pub offset: f64,
    /// Current encoder quality description, or starting.
    pub quality: String,
    /// Audio is held for first-clip loading, independently of the user's playing flag.
    #[serde(default)]
    pub held: bool,
    /// Front favicon deck (a/b), or null when theme video is visible.
    #[serde(default)]
    pub front: Option<String>,
}

/// Working blocking client for Python's unauthenticated 127.0.0.1 control API.
/// This type never reads a T3 token or runtime/settings file.
pub struct ControlClient {
    client: Client,
    port: u16,
}

impl ControlClient {
    /// Port `glowtube.ctl` transport: five-second total timeout, loopback only.
    /// Disable environment proxies and redirects so commands cannot leave the helper endpoint.
    pub fn new(port: u16) -> Result<Self> {
        if port == 0 {
            return Err(Error::InvalidInput("control port must be nonzero"));
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(CONTROL_TIMEOUT)
            .build()
            .map_err(|_| Error::Protocol("could not initialize control client"))?;
        Ok(Self { client, port })
    }

    /// Port `glowtube.ctl`: `GET /<command>?v=<form-url-encoded-arg>`, return the server's JSON.
    /// Preserve unknown JSON fields. Only status is read-only; all other commands mutate the helper.
    /// Errors show status/static context only, never command URLs, user arguments or response bodies.
    pub fn send(&self, command: CtlCommand, argument: &str) -> Result<Value> {
        let url = format!("http://127.0.0.1:{}/{}", self.port, command.path());
        let response = self
            .client
            .get(url)
            .query(&[("v", argument)])
            .send()
            .map_err(|_| Error::Protocol("control request failed; is the helper running?"))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
            });
        }
        response
            .json()
            .map_err(|_| Error::Protocol("helper returned invalid JSON"))
    }
}

/// Port `glowtube.command`: validate/apply one command under player synchronization.
/// Return false for unknown commands; status must not wake writes, all successful mutations do.
pub fn command(player: &mut Player, command: &str, argument: &str) -> Result<bool> {
    use crate::player::AddMode;
    fn number(argument: &str, default: Option<f64>) -> Result<f64> {
        let value = if argument.is_empty() {
            default
        } else {
            argument.trim().parse().ok()
        };
        value
            .filter(|x| x.is_finite())
            .ok_or(Error::InvalidInput("expected finite number"))
    }
    match command {
        "status" => return Ok(true),
        "toggle" => player.set_playing(!player.playing)?,
        "play" | "pause" => player.set_playing(command == "play")?,
        "next" => player.jump(1)?,
        "prev" => player.jump(-1)?,
        "seek" => player.seek(number(argument, Some(10.0))?)?,
        "vol" => player.set_volume(number(argument, None)?)?,
        "chaos" => player.chaos = number(argument, None)?.clamp(0.0, 11.0),
        "offset" => {
            let offset = player.offset + number(argument, None)?;
            if !offset.is_finite() {
                return Err(Error::InvalidInput("offset overflow"));
            }
            player.offset = offset;
        }
        "add" => player.add(argument.into(), AddMode::Now)?,
        "queue" => player.add(argument.into(), AddMode::End)?,
        "playnext" => {
            let offset = if argument.is_empty() {
                1
            } else {
                argument
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| Error::InvalidInput("expected integer"))?
            };
            if offset > 0 {
                player.play_next(offset as usize)?;
            }
        }
        "favicon" => {
            player.favicon_enabled = argument != "off";
            player.invalidate();
        }
        "holdwrites" => {
            let seconds = number(argument, Some(2.0))?;
            let now = Instant::now();
            player.hold_until = Some(
                now.checked_add(
                    Duration::try_from_secs_f64(seconds.max(0.0))
                        .map_err(|_| Error::InvalidInput("hold too long"))?,
                )
                .ok_or(Error::InvalidInput("hold too long"))?,
            );
        }
        "quit" => player.running = false,
        _ => return Ok(false),
    }
    player.dirty = true;
    Ok(true)
}

/// Request channel boundary for `glowtube.serve`; player ownership stays on orchestration's thread.
pub struct ControlRequest {
    /// Decoded URL path without surrounding slashes, e.g. status; unknown paths produce 404.
    pub command: String,
    /// Decoded query parameter v, default empty.
    pub argument: String,
    /// One reply per request. Send a snapshot on success, Http 404 for an unknown command,
    /// or another redacted error; a disconnected receiver must not stop the player.
    pub reply: SyncSender<Result<ControlStatus>>,
}

/// Owned loopback listener and request thread, with bounded headers and interruptible IO.
pub struct ControlServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    address: SocketAddrV4,
}
impl ControlServer {
    /// Bind exactly 127.0.0.1; port zero allocates an isolated ephemeral test endpoint.
    pub fn start(address: SocketAddrV4, requests: Sender<ControlRequest>) -> Result<Self> {
        if *address.ip() != Ipv4Addr::LOCALHOST {
            return Err(Error::InvalidInput("control server must bind 127.0.0.1"));
        }
        let listener = std::net::TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = match listener.local_addr()? {
            std::net::SocketAddr::V4(address) => address,
            _ => return Err(Error::Protocol("invalid control address")),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::Builder::new()
            .name("distraction-control".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = respond(&mut stream, &requests, &stopping);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
            address,
        })
    }
    /// Actual loopback address (including the assigned ephemeral port).
    pub fn address(&self) -> SocketAddrV4 {
        self.address
    }
    /// Interrupt pending reads/replies and join the listener, closing its owned socket.
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| Error::Protocol("control server failed"))?;
        }
        Ok(())
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn respond(
    stream: &mut std::net::TcpStream,
    requests: &Sender<ControlRequest>,
    stop: &AtomicBool,
) -> Result<()> {
    use std::io::{Read, Write};
    // Windows can inherit nonblocking mode from the listener; use bounded blocking IO here.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(25)))?;
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let mut header = Vec::new();
    let mut chunk = [0; 512];
    let target = loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= deadline || header.len() >= 16384 {
            break Err(400);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(count) => header.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        if header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let first = header
                .split(|byte| *byte == b'\n')
                .next()
                .unwrap_or_default();
            let mut parts = std::str::from_utf8(first).unwrap_or("").split_whitespace();
            if parts.next() != Some("GET") {
                break Err(501);
            }
            let target = parts.next().and_then(parse_request);
            break target.ok_or(400);
        }
    };
    let result = match target {
        Err(status) => Err(Error::Http { status }),
        Ok((command, argument)) => {
            let (reply, receive) = mpsc::sync_channel(1);
            if requests
                .send(ControlRequest {
                    command,
                    argument,
                    reply,
                })
                .is_err()
            {
                return Ok(());
            }
            loop {
                match receive.recv_timeout(Duration::from_millis(25)) {
                    Ok(answer) => break answer,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                            break Err(Error::Http { status: 503 });
                        }
                    }
                    Err(_) => break Err(Error::Http { status: 503 }),
                }
            }
        }
    };
    let (status, body) = match result {
        Ok(status) => (
            200,
            serde_json::to_vec(&status).map_err(|_| Error::Protocol("invalid control JSON"))?,
        ),
        Err(Error::Http { status }) => (status, vec![]),
        Err(Error::InvalidInput(_)) => (400, vec![]),
        Err(_) => (500, vec![]),
    };
    write!(
        stream,
        "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    Ok(())
}

fn parse_request(target: &str) -> Option<(String, String)> {
    if !target.starts_with('/') || target.len() > 16384 {
        return None;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let url = reqwest::Url::parse(&format!("http://127.0.0.1/?{query}")).ok()?;
    let argument = url
        .query_pairs()
        .filter(|(key, value)| key == "v" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
        .last()
        .unwrap_or_default();
    Some((path.trim_matches('/').into(), argument))
}

/// Dispatch on the owner thread and reply without panicking when a client disconnects.
pub fn handle_request(player: &mut Player, request: ControlRequest) {
    let result = command(player, &request.command, &request.argument).and_then(|known| {
        if known {
            Ok(player.status())
        } else {
            Err(Error::Http { status: 404 })
        }
    });
    let _ = request.reply.try_send(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    // Isolated ephemeral loopback listener: never touches the running helper or user's T3 state.
    fn mock_status(status: u16, body: &'static str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(CONTROL_TIMEOUT)).unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            write!(socket, "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (port, handle)
    }

    #[test]
    fn status_uses_pythons_get_path_and_query_and_preserves_json() {
        let (port, server) = mock_status(
            200,
            r#"{"track":null,"playing":false,"status":"","time":0.0,"queue":[],"index":0,"volume":0.8,"chaos":11.0,"offset":0.16,"quality":"starting","future":true}"#,
        );
        let value = ControlClient::new(port)
            .unwrap()
            .send(CtlCommand::Status, "")
            .unwrap();
        assert_eq!(value["future"], true);
        let status: ControlStatus = serde_json::from_value(value).unwrap();
        assert_eq!(status.track, None);
        assert_eq!(status.queue, Vec::<String>::new());
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /status?v= HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }

    #[test]
    fn control_errors_exclude_response_contents() {
        let (port, server) = mock_status(500, "private response body");
        let error = ControlClient::new(port)
            .unwrap()
            .send(CtlCommand::Status, "")
            .unwrap_err();
        assert_eq!(error.to_string(), "HTTP 500");
        server.join().unwrap();
        let (port, server) = mock_status(200, "invalid JSON");
        assert!(matches!(
            ControlClient::new(port)
                .unwrap()
                .send(CtlCommand::Status, ""),
            Err(Error::Protocol(_))
        ));
        server.join().unwrap();
        assert!(ControlClient::new(0).is_err());
    }
}
