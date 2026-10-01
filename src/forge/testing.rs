//! Scripted loopback HTTP server for forge tests.

use std::collections::VecDeque;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::Mutex;

pub(crate) enum Reply {
    Ok {
        body: &'static str,
        etag: Option<&'static str>,
    },
    NotModified {
        etag: &'static str,
    },
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub if_none_match: Option<String>,
}

pub(crate) struct StubServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl StubServer {
    pub(crate) fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&requests);
        let mut queue: VecDeque<Reply> = replies.into();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Ok(peek) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(peek);
                let mut if_none_match = None;
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line
                        .strip_prefix("if-none-match:")
                        .or_else(|| line.strip_prefix("If-None-Match:"))
                    {
                        if_none_match = Some(value.trim().to_string());
                    }
                }
                recorder
                    .lock()
                    .expect("request log poisoned")
                    .push(Recorded { if_none_match });

                let response = match queue.pop_front() {
                    Some(Reply::Ok { body, etag }) => {
                        let etag_header = etag
                            .map(|e| format!("ETag: {e}\r\n"))
                            .unwrap_or_else(String::new);
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{etag_header}Content-Length: {}\r\n\r\n{body}",
                            body.len()
                        )
                    }
                    Some(Reply::NotModified { etag }) => {
                        format!("HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\n\r\n")
                    }
                    None => "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n"
                        .to_string(),
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        Self { addr, requests }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("request log poisoned").clone()
    }
}
