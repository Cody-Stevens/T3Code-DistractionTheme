#![allow(dead_code)] // Shared by independent integration-test executables.

use dt_t3::T3Paths;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub fn paths(root: &std::path::Path) -> T3Paths {
    T3Paths {
        home: root.into(),
        runtime_file: root.join("userdata/server-runtime.json"),
        token_file: root.join("secrets/t3-token"),
        themes_dir: root.join("userdata/themes"),
        settings_file: root.join("userdata/settings.json"),
        cli: root.join("bin/t3"),
    }
}

pub fn runtime(paths: &T3Paths, origin: &str) {
    std::fs::create_dir_all(paths.runtime_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.runtime_file,
        serde_json::to_vec(&serde_json::json!({"origin": origin, "pid": 42})).unwrap(),
    )
    .unwrap();
}

pub fn fake_token(paths: &T3Paths) {
    std::fs::create_dir_all(paths.token_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.token_file, "  fake-test-bearer\n").unwrap();
}

#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: String,
    pub body: Value,
}

pub struct MockServer {
    pub origin: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    pub fn new(respond: impl Fn(&Request) -> (u16, String) + Send + 'static) -> Self {
        Self::with_headers(move |request| {
            let (status, body) = respond(request);
            (status, Vec::new(), body)
        })
    }

    pub fn with_headers(
        respond: impl Fn(&Request) -> (u16, Vec<(String, String)>, String) + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, stopping) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("mock accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut headers = Vec::new();
                let mut byte = [0; 1];
                while !headers.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    headers.push(byte[0]);
                    assert!(headers.len() < 16_384);
                }
                let headers = String::from_utf8(headers).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let request = Request {
                    method: first.next().unwrap().into(),
                    path: first.next().unwrap().into(),
                    body: if body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&body).unwrap()
                    },
                    headers,
                };
                let (status, headers, response) = respond(&request);
                seen.lock().unwrap().push(request);
                let extra: String = headers
                    .into_iter()
                    .map(|(key, value)| format!("{key}: {value}\r\n"))
                    .collect();
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} Mock\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}
