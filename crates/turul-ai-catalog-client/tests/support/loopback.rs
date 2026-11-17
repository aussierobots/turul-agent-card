//! One scripted loopback HTTP/1.1 server for client tests (std::net only).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How the body length is announced.
#[derive(Clone)]
pub enum Length {
    /// `Content-Length` equals the body length.
    Exact,
    /// No `Content-Length`; the connection closes after the body.
    Omit,
    /// `Content-Length` announces this value whatever the body holds.
    Announce(u64),
}

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub length: Length,
    /// Wait before sending anything.
    pub delay: Duration,
    /// Send the head, then wait this long before the body.
    pub stall_after_head: Option<Duration>,
    /// Send the body one byte at a time, waiting this long before each byte.
    pub drip: Option<Duration>,
}

impl Reply {
    pub fn new(status: u16) -> Self {
        Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            length: Length::Exact,
            delay: Duration::ZERO,
            stall_after_head: None,
            drip: None,
        }
    }
    pub fn ok(content_type: &str, body: &[u8]) -> Self {
        Reply::new(200)
            .header("Content-Type", content_type)
            .body(body)
    }
    pub fn redirect(status: u16, location: &str) -> Self {
        Reply::new(status).header("Location", location)
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn body(mut self, body: &[u8]) -> Self {
        self.body = body.to_vec();
        self
    }
    pub fn length(mut self, length: Length) -> Self {
        self.length = length;
        self
    }
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
    pub fn stall_after_head(mut self, d: Duration) -> Self {
        self.stall_after_head = Some(d);
        self
    }
    pub fn drip(mut self, d: Duration) -> Self {
        self.drip = Some(d);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Logged {
    pub request_line: String,
    /// Header lines, names lower-cased.
    pub headers: Vec<(String, String)>,
}

impl Logged {
    pub fn path(&self) -> &str {
        self.request_line.split(' ').nth(1).unwrap_or("")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct Server {
    port: u16,
    script: Arc<Mutex<VecDeque<Reply>>>,
    log: Arc<Mutex<Vec<Logged>>>,
}

impl Server {
    /// Binds 127.0.0.1:0 and serves scripted replies in order, one per request.
    pub fn start(replies: Vec<Reply>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let script = Arc::new(Mutex::new(VecDeque::from(replies)));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (s, l) = (script.clone(), log.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (s, l) = (s.clone(), l.clone());
                thread::spawn(move || serve(stream, s, l));
            }
        });
        Server { port, script, log }
    }

    /// A server that logs requests and never answers with anything scripted.
    pub fn silent() -> Server {
        Server::start(Vec::new())
    }

    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
    pub fn push(&self, reply: Reply) {
        self.script.lock().unwrap().push_back(reply);
    }
    pub fn log(&self) -> Vec<Logged> {
        self.log.lock().unwrap().clone()
    }
}

fn serve(mut stream: TcpStream, script: Arc<Mutex<VecDeque<Reply>>>, log: Arc<Mutex<Vec<Logged>>>) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("").to_string();
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    log.lock().unwrap().push(Logged {
        request_line,
        headers,
    });

    let Some(reply) = script.lock().unwrap().pop_front() else {
        return; // unscripted: close without answering
    };
    thread::sleep(reply.delay);
    let mut out = format!("HTTP/1.1 {} X\r\n", reply.status);
    for (n, v) in &reply.headers {
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    match reply.length {
        Length::Exact => out.push_str(&format!("Content-Length: {}\r\n", reply.body.len())),
        Length::Announce(n) => out.push_str(&format!("Content-Length: {n}\r\n")),
        Length::Omit => {}
    }
    out.push_str("Connection: close\r\n\r\n");
    if stream.write_all(out.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();
    if let Some(d) = reply.stall_after_head {
        thread::sleep(d);
    }
    match reply.drip {
        Some(d) => {
            for byte in &reply.body {
                thread::sleep(d);
                if stream
                    .write_all(&[*byte])
                    .and_then(|()| stream.flush())
                    .is_err()
                {
                    return;
                }
            }
        }
        None => {
            let _ = stream.write_all(&reply.body);
            let _ = stream.flush();
        }
    }
}
