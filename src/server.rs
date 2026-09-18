//! Read-only loopback reports, including diagnostic/candidate excerpts. No arbitrary
//! source-file access, credentials, command execution or uploads.
use anyhow::{Result, ensure};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

pub fn run(root: &Path, port: u16) -> Result<()> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    eprintln!("JevGate report API: http://{}", listener.local_addr()?);
    loop {
        crate::cancellation::check()?;
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = respond(root, &mut stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn respond(root: &Path, stream: &mut TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = [0u8; 8192];
    let mut size = 0;
    while size < buffer.len() {
        let count = stream.read(&mut buffer[size..])?;
        if count == 0 {
            break;
        }
        size += count;
        if buffer[..size].windows(4).any(|b| b == b"\r\n\r\n") {
            break;
        }
    }
    let header = std::str::from_utf8(&buffer[..size])?;
    ensure!(header.ends_with("\r\n\r\n"), "Incomplete HTTP request");
    let first = header.lines().next().unwrap_or_default();
    let parts: Vec<_> = first.split_whitespace().collect();
    let host = header.lines().find_map(|line| {
        line.split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.trim())
    });
    let allowed_host = host.is_some_and(|h| {
        h == "localhost"
            || h == "127.0.0.1"
            || h.starts_with("localhost:")
            || h.starts_with("127.0.0.1:")
    });
    let origin = header
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("origin:"));
    let (status, body) = if parts.len() != 3 || parts[0] != "GET" || origin || !allowed_host {
        (
            "403 Forbidden",
            json!({"error":"Only local non-browser GET requests are supported"}),
        )
    } else {
        route(root, parts[1])
    };
    let body = serde_json::to_vec(&body)?;
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    Ok(())
}

pub fn route(root: &Path, route: &str) -> (&'static str, serde_json::Value) {
    let mut report = match crate::storage::read_latest(root) {
        Ok(report) => report,
        Err(_) => {
            return (
                "503 Service Unavailable",
                json!({"error":"No compatible snapshot available"}),
            );
        }
    };
    if report.watcher_pid.is_some() && !crate::storage::writer_active(root) {
        report.watcher_pid = None;
        report.errors.push("Watcher is no longer active".into());
    }
    for file in &mut report.files {
        let current = crate::inventory::read_source(&root.join(&file.path), 1024 * 1024)
            .is_ok_and(|s| crate::schema::hash(s.as_bytes()) == file.source_hash);
        let context_current = file.context_files.iter().all(|c| {
            crate::inventory::read_source(&root.join(&c.path), 1024 * 1024)
                .is_ok_and(|s| crate::schema::hash(s.as_bytes()) == c.source_hash)
        });
        if file.status != crate::schema::Status::Skipped && (!current || !context_current) {
            file.status = crate::schema::Status::Error;
            file.error = Some("Stored evidence is stale".into());
        }
    }
    report.update_status();
    match route {
        "/snapshot" => ("200 OK", json!(report)),
        "/evidence" => (
            "200 OK",
            json!({"generation":report.generation,"files":report.files.iter().map(|f| json!({"path":f.path,"source_hash":f.source_hash,"context":f.context_files,"findings":f.findings,"current":f.error.is_none()})).collect::<Vec<_>>()}),
        ),
        "/context-requests" => (
            "200 OK",
            json!({"generation":report.generation,"requests":report.files.iter().flat_map(|f| f.context_requests.iter().map(|r| json!({"path":f.path,"request":r}))).collect::<Vec<_>>()}),
        ),
        _ if route.starts_with("/changes?since=") => {
            match route.trim_start_matches("/changes?since=").parse::<u64>() {
                Ok(since) if since <= report.generation => match crate::storage::history(
                    root,
                    since,
                    if report.settled {
                        report.generation
                    } else {
                        report.generation.saturating_sub(1)
                    },
                ) {
                    Ok(history) => (
                        "200 OK",
                        json!({"generation":if report.settled { report.generation } else { report.generation.saturating_sub(1) },"pending_generation":if report.settled { None } else { Some(report.generation) },"changes":history}),
                    ),
                    Err(_) => (
                        "409 Conflict",
                        json!({"error":"History gap; fetch /snapshot and reset the cursor", "generation":report.generation}),
                    ),
                },
                _ => (
                    "400 Bad Request",
                    json!({"error":"Invalid generation cursor"}),
                ),
            }
        }
        _ => (
            "404 Not Found",
            json!({"error":"Use /snapshot, /changes?since=N, /evidence or /context-requests"}),
        ),
    }
}
