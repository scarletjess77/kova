//! Kova engine: everything with side effects. Storage, downloads, running cores,
//! talking to them, and the system proxy. The UI layer (Tauri) only calls [`Engine`].

pub mod backup;
pub mod cores;
pub mod diag;
pub mod engine;
pub mod fetch;
pub mod geodata;
pub mod mihomo_api;
pub mod process;
pub mod secret;
pub mod service;
pub mod sitecheck;
pub mod smart;
pub mod store;
pub mod sysproxy;
#[cfg(target_os = "linux")]
pub mod tun_linux;

pub use engine::{Engine, Event, NodeView, Status};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{}", net_message(.0))]
    Net(#[from] reqwest::Error),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Core(String),
    #[error("the config is invalid: {0}")]
    Config(String),
    #[error("{0}")]
    Parse(#[from] kova_core::subscription::ParseError),
    #[error("{0}")]
    Other(String),
}

fn net_message(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the server took too long to answer".into()
    } else if e.is_connect() {
        "could not connect to the server. Check the address and your connection".into()
    } else if e.is_decode() {
        "the server sent something that is not a subscription".into()
    } else {
        format!("network error: {e}")
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
