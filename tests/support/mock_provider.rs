//! A provider on 127.0.0.1 for tests: each request gets the reply a closure
//! computes from it, over plain HTTP/1.1 with one request per connection,
//! and every request is recorded; and scripted answers to any request.
use serde_json::Value;
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
    pub fn json(&self) -> Value {
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
    pub fn json(status: u16, body: &Value) -> Self {
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
    let Some(request) = read_request(&stream) else {
        return;
    };
    log.lock().unwrap().push(request.clone());
    let reply = respond(&request);
    std::thread::sleep(reply.delay);
    write_reply(stream, &reply);
}

/// The request on `stream`, or none when the client closed it first.
fn read_request(stream: &TcpStream) -> Option<Received> {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return None;
    }
    let mut words = line.split_whitespace();
    let (method, path) = (
        words.next().unwrap_or_default().to_owned(),
        words.next().unwrap_or_default().to_owned(),
    );
    let headers = read_headers(&mut reader);
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    Some(Received {
        method,
        path,
        headers,
        body: String::from_utf8(body).unwrap(),
    })
}

/// The header lines, up to the blank line that ends them.
fn read_headers(reader: &mut impl BufRead) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            return headers;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
}

/// Send `reply`, closing the connection after it. The client may have given
/// up waiting; a failed write is its business.
fn write_reply(mut stream: TcpStream, reply: &Reply) {
    let mut head = format!(
        "HTTP/1.1 {} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let _ = stream.write_all(format!("{head}\r\n{}", reply.body).as_bytes());
}

/// Levels: 0 answers the bottom of every scale (clear), 1 the middle (consider,
/// or a note where the middle says the code is fine), 2 the top (review),
/// 3 spreads probability (uncertain), 4 leans to the top without reaching review.
pub fn answer(request: &Value, level: usize) -> Value {
    let answers = request["questions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, q)| (name.clone(), typed_answer(q, level)))
        .collect::<serde_json::Map<_, _>>();
    serde_json::json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":0}})
}

/// A valid answer of the question's type at `level` (see [`answer`]).
fn typed_answer(question: &Value, level: usize) -> Value {
    match question["type"].as_str().unwrap() {
        "noul" => {
            let noul = [0.05, 0.5, 0.95, 0.5, 0.5][level];
            serde_json::json!({"type":"noul","noul":noul})
        }
        "score" => {
            let p = [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.4, 0.2, 0.4],
                [0.1, 0.3, 0.6],
            ][level];
            serde_json::json!({"type":"score","score":p[1] + 2.0 * p[2],"confidence":1.0,
                "probabilities":{"0":p[0],"1":p[1],"2":p[2]}})
        }
        _ => choice_answer(question["criteria"].as_object().unwrap()),
    }
}

/// A certain Choice of `none` when offered, else the first option.
fn choice_answer(options: &serde_json::Map<String, Value>) -> Value {
    let chosen = if options.contains_key("none") {
        "none"
    } else {
        options.keys().next().unwrap()
    };
    let probabilities: serde_json::Map<_, _> = options
        .keys()
        .map(|k| {
            (
                k.clone(),
                serde_json::json!(if k == chosen { 1.0 } else { 0.0 }),
            )
        })
        .collect();
    serde_json::json!({"type":"choice","choice":chosen,"confidence":1.0,"probabilities":probabilities})
}
