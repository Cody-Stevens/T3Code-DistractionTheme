//! Python-compatible client and planned server, porting `glowtube.ctl/command/serve`.

use crate::{cli::CtlCommand, player::Player};
use dt_core::{Error, Result, constants::CONTROL_TIMEOUT};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    net::SocketAddrV4,
    sync::mpsc::{Sender, SyncSender},
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
pub fn command(_player: &mut Player, _command: &str, _argument: &str) -> Result<bool> {
    todo!("port command dispatch")
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

/// Port `glowtube.serve`'s server owner; stop/join request threads on shutdown and drop.
pub struct ControlServer {
    _private: (),
}

impl ControlServer {
    /// Port `serve`: bind only 127.0.0.1 (default 47873), GET path command and v query argument.
    /// Return the exact ControlStatus fields on success, HTTP 404 for an unknown command.
    /// Use a request channel so slow downloads/encodes never execute on the listener thread.
    pub fn start(_address: SocketAddrV4, _requests: Sender<ControlRequest>) -> Result<Self> {
        todo!("start loopback server")
    }

    /// Port `server.shutdown`: stop accepting requests and join owned threads without a live probe.
    pub fn stop(&mut self) -> Result<()> {
        todo!("stop control server")
    }
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
