//! Private, opt-in IPC between the game and a local MCP sidecar.

use bevy::prelude::Resource;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_REQUEST: usize = 64 * 1024;
const MAX_RESPONSE: usize = 1024 * 1024;
const QUEUE_DEPTH: usize = 16;
const IO_DEADLINE: Duration = Duration::from_secs(2);
const ACCEPT_PAUSE: Duration = Duration::from_millis(10);

pub(crate) struct Request {
    pub op: String,
    pub lease: Option<String>,
    pub arguments: Value,
}

pub(crate) struct Pending {
    pub request: Request,
    pub deadline: Instant,
    pub reply: SyncSender<Result<Value, String>>,
}

#[derive(Resource)]
pub(crate) struct Bridge {
    incoming: Mutex<Receiver<Pending>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    endpoint: PathBuf,
    #[cfg(unix)]
    endpoint_identity: (u64, u64),
    #[cfg(not(unix))]
    endpoint_contents: Vec<u8>,
}

impl Bridge {
    pub(crate) fn start(endpoint: PathBuf) -> Result<Self, String> {
        if let Some(parent) = endpoint
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("MCP endpoint directory creation failed: {e}"))?;
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("MCP loopback bind failed: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("MCP listener setup failed: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("MCP listener address failed: {e}"))?
            .port();

        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|e| format!("MCP token generation failed: {e}"))?;
        let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let contents = serde_json::to_vec(&json!({ "port": port, "token": token }))
            .map_err(|e| format!("MCP endpoint encoding failed: {e}"))?;

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&endpoint)
            .map_err(|e| format!("MCP endpoint creation failed: {e}"))?;
        #[cfg(unix)]
        let endpoint_identity = {
            use std::os::unix::fs::MetadataExt;
            let meta = match file.metadata() {
                Ok(meta) => meta,
                Err(e) => {
                    let _ = fs::remove_file(&endpoint);
                    return Err(format!("MCP endpoint metadata failed: {e}"));
                }
            };
            (meta.dev(), meta.ino())
        };
        if let Err(e) = file.write_all(&contents) {
            let _ = fs::remove_file(&endpoint);
            return Err(format!("MCP endpoint write failed: {e}"));
        }
        drop(file);

        let (send, receive) = mpsc::sync_channel(QUEUE_DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = match thread::Builder::new()
            .name("sta64-mcp-bridge".into())
            .spawn(move || serve(listener, token, send, worker_stop))
        {
            Ok(worker) => worker,
            Err(e) => {
                let _ = fs::remove_file(&endpoint);
                return Err(format!("MCP listener thread failed: {e}"));
            }
        };
        Ok(Self {
            incoming: Mutex::new(receive),
            stop,
            worker: Some(worker),
            endpoint,
            #[cfg(unix)]
            endpoint_identity,
            #[cfg(not(unix))]
            endpoint_contents: contents,
        })
    }

    /// Poll once from a game system; this never waits for the IPC thread.
    pub(crate) fn next(&self) -> Option<Pending> {
        let incoming = self.incoming.try_lock().ok()?;
        incoming.try_recv().ok()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // Do not remove a path that another process replaced after startup.
        #[cfg(unix)]
        let owned = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&self.endpoint)
                .map(|meta| (meta.dev(), meta.ino()) == self.endpoint_identity)
                .unwrap_or(false)
        };
        #[cfg(not(unix))]
        let owned = fs::read(&self.endpoint)
            .map(|bytes| bytes == self.endpoint_contents)
            .unwrap_or(false);
        if owned {
            let _ = fs::remove_file(&self.endpoint);
        }
    }
}

fn serve(listener: TcpListener, token: String, send: SyncSender<Pending>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) if peer.ip().is_loopback() => handle(stream, &token, &send),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_PAUSE),
            Err(_) => break,
        }
    }
}

fn handle(mut stream: TcpStream, token: &str, send: &SyncSender<Pending>) {
    let deadline = Instant::now() + IO_DEADLINE;
    let result = read_line(&mut stream, deadline).and_then(|line| parse_request(&line, token));
    let request = match result {
        Ok(request) => request,
        Err(error) => {
            let _ = write_response(&mut stream, Err(error), Instant::now() + IO_DEADLINE);
            return;
        }
    };
    let (reply, receive) = mpsc::sync_channel(1);
    let deadline = Instant::now() + IO_DEADLINE;
    if send
        .try_send(Pending {
            request,
            deadline,
            reply,
        })
        .is_err()
    {
        let _ = write_response(
            &mut stream,
            Err("MCP request queue unavailable".into()),
            Instant::now() + IO_DEADLINE,
        );
        return;
    }
    let response = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or_else(|_| Err("MCP game response timed out".into()));
    let _ = write_response(&mut stream, response, Instant::now() + IO_DEADLINE);
}

fn read_line(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, String> {
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("MCP request timed out")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| "MCP read setup failed")?;
        let n = stream
            .read(&mut buf)
            .map_err(|_| "MCP request read failed")?;
        if n == 0 {
            return Err("MCP request ended before newline".into());
        }
        if let Some(end) = buf[..n].iter().position(|&b| b == b'\n') {
            if line.len() + end > MAX_REQUEST {
                return Err("MCP request too large".into());
            }
            line.extend_from_slice(&buf[..end]);
            return Ok(line);
        }
        if line.len() + n > MAX_REQUEST {
            return Err("MCP request too large".into());
        }
        line.extend_from_slice(&buf[..n]);
    }
}

fn parse_request(line: &[u8], token: &str) -> Result<Request, String> {
    let value: Value = serde_json::from_slice(line).map_err(|_| "Invalid MCP request JSON")?;
    let object = value.as_object().ok_or("MCP request must be an object")?;
    if object.get("token").and_then(Value::as_str) != Some(token) {
        return Err("MCP authentication failed".into());
    }
    let op = object
        .get("op")
        .and_then(Value::as_str)
        .filter(|op| !op.is_empty())
        .ok_or("MCP op is required")?
        .to_owned();
    let lease = match object.get("lease") {
        None | Some(Value::Null) => None,
        Some(Value::String(lease)) => Some(lease.clone()),
        Some(_) => return Err("MCP lease must be a string".into()),
    };
    let arguments = match object.get("arguments") {
        None => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => return Err("MCP arguments must be an object".into()),
    };
    Ok(Request {
        op,
        lease,
        arguments,
    })
}

fn write_response(
    stream: &mut TcpStream,
    response: Result<Value, String>,
    deadline: Instant,
) -> Result<(), String> {
    let value = match response {
        Ok(data) => json!({ "ok": true, "data": data }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    let mut bytes = serde_json::to_vec(&value).map_err(|_| "MCP response encoding failed")?;
    if bytes.len() > MAX_RESPONSE {
        bytes = b"{\"ok\":false,\"error\":\"MCP response too large\"}".to_vec();
    }
    bytes.push(b'\n');
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("MCP response timed out")?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(|_| "MCP write setup failed")?;
        let n = stream
            .write(&bytes[written..])
            .map_err(|_| "MCP response write failed")?;
        if n == 0 {
            return Err("MCP response write failed".into());
        }
        written += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_request_round_trip_and_endpoint_cleanup() {
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).unwrap();
        let endpoint = std::env::temp_dir().join(format!(
            "sta64-mcp-{}-{}.json",
            std::process::id(),
            u64::from_le_bytes(suffix)
        ));
        let bridge = Bridge::start(endpoint.clone()).unwrap();
        let descriptor: Value = serde_json::from_slice(&fs::read(&endpoint).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let port = descriptor["port"].as_u64().unwrap() as u16;
        let token = descriptor["token"].as_str().unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(IO_DEADLINE)).unwrap();
        let request = json!({ "token": token, "op": "status" });
        writeln!(stream, "{request}").unwrap();

        let deadline = Instant::now() + IO_DEADLINE;
        let pending = loop {
            if let Some(pending) = bridge.next() {
                break pending;
            }
            assert!(Instant::now() < deadline, "bridge did not deliver request");
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(pending.request.op, "status");
        assert_eq!(pending.request.arguments, json!({}));
        pending.reply.send(Ok(json!({ "ready": true }))).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&response).unwrap(),
            json!({ "ok": true, "data": { "ready": true } })
        );
        drop(bridge);
        assert!(!endpoint.exists());
    }

    #[test]
    fn rejects_wrong_capability_without_queueing() {
        let result = parse_request(br#"{"token":"wrong","op":"status"}"#, "right");
        assert!(matches!(result, Err(message) if message == "MCP authentication failed"));
    }
}
