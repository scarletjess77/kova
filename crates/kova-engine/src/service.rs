//! Talking to the Kova service: a small privileged process that runs Mihomo with the
//! rights TUN needs, so the app itself never has to run as administrator.
//!
//! One JSON request per line and one JSON response per line, over a local named pipe.
//! The service only ever runs its own Mihomo binary (downloaded and verified by
//! itself) with a config it sanitises; see the kova-service crate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kova_core::options::GeoSource;
use serde::{Deserialize, Serialize};

use crate::Error;

pub const PIPE_NAME: &str = r"\\.\pipe\kova-service";
pub const PROTOCOL_VERSION: u32 = 1;

/// The pipe to use; tests point this at a private instance.
pub fn pipe_name() -> String {
    std::env::var("KOVA_SERVICE_PIPE").unwrap_or_else(|_| PIPE_NAME.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello,
    /// Start Mihomo with `config`. The service stops it again if `owner_pid` exits.
    Start { config: String, geodata: GeoSource, owner_pid: u32 },
    Stop,
    Status,
    /// Log lines with a sequence number greater than `since`.
    Logs { since: u64 },
    /// Download (or update) the service's own Mihomo.
    InstallCore,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Response {
    pub ok: bool,
    pub error: Option<String>,
    pub version: Option<String>,
    pub protocol: Option<u32>,
    pub core_version: Option<String>,
    pub running: bool,
    /// Why the core stopped, if it stopped on its own.
    pub exit: Option<String>,
    pub lines: Vec<(u64, String)>,
    /// Sequence number of the newest log line, so a client can skip older ones.
    pub log_seq: u64,
    /// SHA-256 of the service executable. Equal to the app's own hash when the service
    /// came from this exact build, which a version number alone can't tell.
    pub exe_sha256: Option<String>,
}

/// SHA-256 of the running executable (computed once).
pub fn own_exe_sha256() -> Option<String> {
    use sha2::{Digest, Sha256};
    static HASH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        let data = std::fs::read(std::env::current_exe().ok()?).ok()?;
        Some(hex::encode(Sha256::digest(&data)))
    })
    .clone()
}

impl Response {
    pub fn ok() -> Self {
        Self { ok: true, ..Default::default() }
    }
    pub fn err(e: impl ToString) -> Self {
        Self { ok: false, error: Some(e.to_string()), ..Default::default() }
    }
}

/// Send one request. `timeout` covers the whole exchange.
pub async fn call(req: &Request, timeout: Duration) -> Result<Response, Error> {
    let fut = async {
        let stream = connect().await?;
        exchange(stream, req).await
    };
    let resp = tokio::time::timeout(timeout, fut).await.map_err(|_| Error::Core("the Kova service did not answer in time".into()))??;
    if resp.ok { Ok(resp) } else { Err(Error::Core(resp.error.unwrap_or_else(|| "the Kova service refused the request".into()))) }
}

#[cfg(windows)]
async fn connect() -> Result<tokio::net::windows::named_pipe::NamedPipeClient, Error> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let name = pipe_name();
    for _ in 0..40 {
        match ClientOptions::new().open(&name) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(e) => return Err(Error::Core(format!("the Kova service is not running ({e})"))),
        }
    }
    Err(Error::Core("the Kova service is busy".into()))
}

#[cfg(not(windows))]
async fn connect() -> Result<tokio::net::UnixStream, Error> {
    Err(Error::Core("the Kova service is only available on Windows for now".into()))
}

async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(stream: S, req: &Request) -> Result<Response, Error> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (r, mut w) = tokio::io::split(stream);
    let mut line = serde_json::to_string(req).expect("json");
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    let mut reply = String::new();
    BufReader::new(r).read_line(&mut reply).await?;
    serde_json::from_str(&reply).map_err(|e| Error::Core(format!("unexpected answer from the Kova service: {e}")))
}

/// Version and protocol of a reachable service, or `None` if none answers.
pub async fn hello() -> Option<Response> {
    call(&Request::Hello, Duration::from_secs(2)).await.ok()
}

/// A Mihomo instance running inside the service.
pub struct RemoteCore {
    tail: Arc<Mutex<Vec<String>>>,
}

impl RemoteCore {
    pub fn new(tail: Arc<Mutex<Vec<String>>>) -> Self {
        Self { tail }
    }

    /// `Some(message)` if the core is no longer running. A single failed request is not
    /// taken as a crash: the pipe can be briefly busy.
    pub async fn exited(&self) -> Option<String> {
        let mut last = String::new();
        for attempt in 0..3 {
            match call(&Request::Status, Duration::from_secs(5)).await {
                Ok(r) if r.running => return None,
                Ok(r) => return Some(r.exit.unwrap_or_else(|| "Mihomo stopped in the Kova service".into())),
                Err(e) => last = e.to_string(),
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
        Some(format!("lost contact with the Kova service: {last}"))
    }

    pub fn last_error(&self) -> String {
        let t = self.tail.lock().unwrap();
        t.iter().rev().find(|l| l.to_ascii_lowercase().contains("error")).or_else(|| t.last()).cloned().unwrap_or_default()
    }

    pub async fn stop(self) {
        stop_remote().await;
    }
}

/// Ask the service to stop Mihomo, retrying briefly: a Mihomo left running there would
/// keep capturing traffic while Kova shows "Disconnected".
pub async fn stop_remote() {
    for _ in 0..3 {
        if call(&Request::Stop, Duration::from_secs(10)).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
