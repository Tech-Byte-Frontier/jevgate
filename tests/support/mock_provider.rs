//! A provider on 127.0.0.1 for tests: each request gets the reply a closure
//! computes from it, over plain HTTP/1.1 with one request per connection,
//! and every request is recorded.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};

/// One request as the server received it.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: String,
}

impl Received {
    /// A header's value, by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// A reply: status, extra headers, body, and how long to wait before sending it.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_owned()));
        self
    }
}

type Respond = dyn Fn(&Received) -> Reply + Send + Sync;

pub struct MockProvider {
    /// `http://127.0.0.1:PORT`, the API root to point JevGate at.
    pub url: String,
    received: Arc<Mutex<Vec<Received>>>,
}

impl MockProvider {
    pub fn start(respond: impl Fn(&Received) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let respond: Arc<Respond> = Arc::new(respond);
        let log = Arc::clone(&received);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, respond) = (Arc::clone(&log), Arc::clone(&respond));
                std::thread::spawn(move || serve(stream, &log, respond.as_ref()));
            }
        });
        Self { url, received }
    }

    /// Every request received so far, in order of arrival.
    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<Received>>, respond: &Respond) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut words = line.split_whitespace();
    let (method, path) = (
        words.next().unwrap_or_default(),
        words.next().unwrap_or_default(),
    );
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request = Received {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body: String::from_utf8(body).unwrap(),
    };
    log.lock().unwrap().push(request.clone());
    let reply = respond(&request);
    std::thread::sleep(reply.delay);
    let mut out = stream;
    let mut head = format!(
        "HTTP/1.1 {} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    // The client may have given up waiting; a failed write is its business.
    let _ = out.write_all(format!("{head}\r\n{}", reply.body).as_bytes());
}
