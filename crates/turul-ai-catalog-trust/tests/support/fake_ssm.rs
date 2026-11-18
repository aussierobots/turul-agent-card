//! A loopback stand-in for the Systems Manager endpoint: `PutParameter` and `GetParameter` over
//! an in-memory map, speaking the JSON 1.1 protocol. std only.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use turul_ai_catalog::json::{Json, read};

#[derive(Default)]
pub struct State {
    /// Parameter name to value.
    pub parameters: HashMap<String, String>,
    /// (`X-Amz-Target`, body) of every request, in order.
    pub requests: Vec<(String, Json)>,
    /// Answer every request with `AccessDeniedException`.
    pub deny: bool,
    /// Names answered as a plain `String` parameter rather than a `SecureString`.
    pub plain: std::collections::HashSet<String>,
}

pub struct FakeSsm {
    port: u16,
    pub state: Arc<Mutex<State>>,
}

impl FakeSsm {
    pub fn start() -> FakeSsm {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let shared = shared.clone();
                thread::spawn(move || serve(stream, shared));
            }
        });
        FakeSsm { port, state }
    }

    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn seed(&self, name: &str, value: &str) {
        self.state
            .lock()
            .unwrap()
            .parameters
            .insert(name.to_string(), value.to_string());
    }

    pub fn value(&self, name: &str) -> Option<String> {
        self.state.lock().unwrap().parameters.get(name).cloned()
    }

    pub fn requests(&self) -> Vec<(String, Json)> {
        self.state.lock().unwrap().requests.clone()
    }
}

fn text(json: &Json) -> Option<&str> {
    match json {
        Json::String(s) => Some(s),
        _ => None,
    }
}

fn member<'a>(json: &'a Json, name: &str) -> Option<&'a Json> {
    match json {
        Json::Object(m) => m.get(name),
        _ => None,
    }
}

fn quoted(s: &str) -> String {
    String::from_utf8(Json::String(s.to_string()).to_canonical_vec().unwrap()).unwrap()
}

fn error(kind: &str) -> (u16, String) {
    (
        400,
        format!(r#"{{"__type":{},"message":"fake"}}"#, quoted(kind)),
    )
}

fn answer(state: &mut State, target: &str, body: &Json) -> (u16, String) {
    if state.deny {
        return error("AccessDeniedException");
    }
    let name = member(body, "Name")
        .and_then(text)
        .unwrap_or("")
        .to_string();
    match target {
        "AmazonSSM.PutParameter" => {
            let overwrite = matches!(member(body, "Overwrite"), Some(Json::Bool(true)));
            if state.parameters.contains_key(&name) && !overwrite {
                return error("ParameterAlreadyExists");
            }
            let value = member(body, "Value").and_then(text).unwrap_or("");
            state.parameters.insert(name, value.to_string());
            (200, r#"{"Version":1,"Tier":"Standard"}"#.to_string())
        }
        "AmazonSSM.GetParameter" => match state.parameters.get(&name) {
            Some(value) => (
                200,
                format!(
                    r#"{{"Parameter":{{"Name":{},"Type":{},"Value":{},"Version":1}}}}"#,
                    quoted(&name),
                    quoted(if state.plain.contains(&name) {
                        "String"
                    } else {
                        "SecureString"
                    }),
                    quoted(value)
                ),
            ),
            None => error("ParameterNotFound"),
        },
        _ => error("UnknownOperationException"),
    }
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => data.extend_from_slice(&buf[..n]),
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while data.len() < head_end + length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => data.extend_from_slice(&buf[..n]),
        }
    }
    let target = header("x-amz-target").unwrap_or_default();
    let (status, body) = match read(&data[head_end..head_end + length]) {
        Ok(json) => {
            let mut state = state.lock().unwrap();
            let reply = answer(&mut state, &target, &json);
            state.requests.push((target, json));
            reply
        }
        Err(_) => error("SerializationException"),
    };
    let out = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/x-amz-json-1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}
