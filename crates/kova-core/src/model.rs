//! Core-neutral description of a proxy node.
//!
//! Every importer (share links, Clash YAML, sing-box JSON) produces [`Node`]s and every
//! generator (Mihomo, Xray) consumes them, so this is the one place that has to know
//! about all protocols.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub server: String,
    pub port: u16,
    #[serde(flatten)]
    pub protocol: Protocol,
    #[serde(default, skip_serializing_if = "Transport::is_tcp")]
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<Tls>,
    #[serde(default = "yes")]
    pub udp: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Protocol {
    Vless {
        uuid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
        /// VLESS encryption. `None` or `"none"` for classic VLESS.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encryption: Option<String>,
    },
    Vmess {
        uuid: String,
        #[serde(default)]
        alter_id: u16,
        #[serde(default = "default_vmess_cipher")]
        cipher: String,
    },
    Trojan {
        password: String,
    },
    Shadowsocks {
        method: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plugin: Option<SsPlugin>,
    },
    Hysteria2 {
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs_password: Option<String>,
        /// Port-hopping range such as `"20000-30000"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ports: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        up_mbps: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        down_mbps: Option<u32>,
    },
    Tuic {
        uuid: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        congestion_control: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        udp_relay_mode: Option<String>,
    },
    Wireguard {
        private_key: String,
        public_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_shared_key: Option<String>,
        /// Interface addresses with prefix, e.g. `10.0.0.2/32`, `fd00::2/128`.
        #[serde(default)]
        addresses: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mtu: Option<u16>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        reserved: Vec<u8>,
    },
    Anytls {
        password: String,
    },
    Socks {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
    },
    Http {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
    },
    /// A Clash proxy type we don't model (ssr, snell, mieru, ...). Kept verbatim so
    /// Mihomo can still run it.
    Other {
        kind: String,
        raw: serde_json::Value,
    },
}

fn default_vmess_cipher() -> String {
    "auto".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SsPlugin {
    /// Clash plugin name: `obfs`, `v2ray-plugin`, `shadow-tls`, `restls`, ...
    pub name: String,
    #[serde(default)]
    pub opts: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "network", rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Tcp,
    Ws {
        #[serde(default)]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
    Grpc {
        #[serde(default)]
        service_name: String,
    },
    H2 {
        #[serde(default)]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
    #[serde(rename = "httpupgrade")]
    HttpUpgrade {
        #[serde(default)]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
    Xhttp {
        #[serde(default)]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// Raw `extra` object from the share link, passed through to Xray.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<serde_json::Value>,
    },
}

impl Transport {
    pub fn is_tcp(&self) -> bool {
        matches!(self, Transport::Tcp)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Transport::Tcp => "TCP",
            Transport::Ws { .. } => "WS",
            Transport::Grpc { .. } => "gRPC",
            Transport::H2 { .. } => "H2",
            Transport::HttpUpgrade { .. } => "HTTPUpgrade",
            Transport::Xhttp { .. } => "XHTTP",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tls {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpn: Vec<String>,
    /// uTLS client fingerprint (`chrome`, `firefox`, `safari`, `random`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reality: Option<Reality>,
    /// The server certificate's SHA-256 (hex, lower case, `,` between several), trusted
    /// instead of the usual check: Xray's `pinnedPeerCertSha256`, Mihomo's `fingerprint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reality {
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spider_x: Option<String>,
}

impl Node {
    /// Short protocol name for the UI.
    pub fn protocol_label(&self) -> String {
        match &self.protocol {
            Protocol::Vless { .. } => "VLESS".into(),
            Protocol::Vmess { .. } => "VMess".into(),
            Protocol::Trojan { .. } => "Trojan".into(),
            Protocol::Shadowsocks { .. } => "Shadowsocks".into(),
            Protocol::Hysteria2 { .. } => "Hysteria2".into(),
            Protocol::Tuic { .. } => "TUIC".into(),
            Protocol::Wireguard { .. } => "WireGuard".into(),
            Protocol::Anytls { .. } => "AnyTLS".into(),
            Protocol::Socks { .. } => "SOCKS5".into(),
            Protocol::Http { .. } => "HTTP".into(),
            Protocol::Other { kind, .. } => kind.to_uppercase(),
        }
    }

    /// Human description of transport and security, e.g. `"XHTTP · Reality"`.
    pub fn transport_label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match &self.protocol {
            Protocol::Hysteria2 { obfs, .. } => {
                parts.push("QUIC".into());
                if let Some(o) = obfs {
                    parts.push(capitalise(o));
                }
            }
            Protocol::Tuic { congestion_control, .. } => {
                parts.push("QUIC".into());
                if let Some(c) = congestion_control {
                    parts.push(c.to_uppercase());
                }
            }
            Protocol::Wireguard { .. } => parts.push("UDP".into()),
            Protocol::Shadowsocks { method, plugin, .. } => {
                parts.push(method.clone());
                if let Some(p) = plugin {
                    parts.push(p.name.clone());
                }
            }
            _ => {
                if !self.transport.is_tcp() {
                    parts.push(self.transport.label().into());
                }
                if let Some(tls) = &self.tls {
                    parts.push(if tls.reality.is_some() { "Reality".into() } else { "TLS".into() });
                }
                if let Protocol::Vless { flow: Some(f), .. } = &self.protocol {
                    if f.contains("vision") {
                        parts.push("Vision".into());
                    }
                }
                if parts.is_empty() {
                    parts.push("TCP".into());
                }
            }
        }
        parts.join(" · ")
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Make node names unique within a list, appending ` 2`, ` 3`, ... to repeats.
/// Cores address nodes by name, so duplicates would silently shadow each other.
pub fn dedupe_names(nodes: &mut [Node]) {
    let mut used = std::collections::HashSet::new();
    for n in nodes.iter_mut() {
        if n.name.trim().is_empty() {
            n.name = format!("{}:{}", n.server, n.port);
        }
        if used.contains(&n.name) {
            let mut i = 2;
            while used.contains(&format!("{} {i}", n.name)) {
                i += 1;
            }
            n.name = format!("{} {i}", n.name);
        }
        used.insert(n.name.clone());
    }
}
