//! Share links: `vless://`, `vmess://`, `trojan://`, `ss://`, `hysteria2://`, `tuic://`,
//! `wireguard://`, `anytls://`, `socks://`, in both directions.

use std::collections::HashMap;

use base64::Engine;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::model::*;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LinkError {
    #[error("unsupported link type `{0}`")]
    Scheme(String),
    #[error("not a share link")]
    NotALink,
    #[error("invalid {scheme} link: {reason}")]
    Invalid { scheme: &'static str, reason: String },
}

fn invalid(scheme: &'static str, reason: impl Into<String>) -> LinkError {
    LinkError::Invalid { scheme, reason: reason.into() }
}

/// Parse one share link into a [`Node`].
pub fn parse_link(s: &str) -> Result<Node, LinkError> {
    let s = s.trim();
    let (scheme, _) = s.split_once("://").ok_or(LinkError::NotALink)?;
    match scheme.to_ascii_lowercase().as_str() {
        "vless" => parse_vless(s),
        "vmess" => parse_vmess(s),
        "trojan" => parse_trojan(s),
        "ss" => parse_ss(s),
        "hysteria2" | "hy2" => parse_hy2(s),
        "tuic" => parse_tuic(s),
        "wireguard" | "wg" => parse_wireguard(s),
        "anytls" => parse_anytls(s),
        "socks" | "socks5" => parse_socks(s),
        other => Err(LinkError::Scheme(other.to_string())),
    }
}

// ---------------------------------------------------------------------------------------
// Generic URI pieces

struct Uri {
    user: Option<String>,
    pass: Option<String>,
    host: String,
    /// The raw port field; may be a hopping spec such as `443,20000-30000`.
    port_spec: String,
    query: HashMap<String, String>,
    name: String,
}

impl Uri {
    fn port(&self, scheme: &'static str) -> Result<u16, LinkError> {
        if self.port_spec.is_empty() {
            return Ok(443);
        }
        let first: String = self.port_spec.chars().take_while(|c| c.is_ascii_digit()).collect();
        first.parse().map_err(|_| invalid(scheme, format!("bad port `{}`", self.port_spec)))
    }

    /// Case-insensitive query lookup over several aliases; empty values count as absent.
    fn q(&self, keys: &[&str]) -> Option<String> {
        keys.iter()
            .find_map(|k| self.query.get(&k.to_ascii_lowercase()))
            .filter(|v| !v.is_empty())
            .cloned()
    }

    fn flag(&self, keys: &[&str]) -> bool {
        matches!(self.q(keys).as_deref(), Some("1" | "true" | "yes"))
    }

    /// The whole userinfo as one secret. Passwords may legally contain an unencoded ':',
    /// which the generic split would otherwise treat as a user/password separator.
    fn secret(&self) -> Option<String> {
        match (&self.user, &self.pass) {
            (Some(u), Some(p)) => Some(format!("{u}:{p}")),
            (Some(u), None) => Some(u.clone()),
            _ => None,
        }
        .filter(|s| !s.is_empty())
    }
}

fn decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn split_uri(s: &str, scheme: &'static str) -> Result<Uri, LinkError> {
    let rest = s.split_once("://").map(|(_, r)| r).ok_or(LinkError::NotALink)?;
    let (rest, name) = match rest.split_once('#') {
        Some((r, n)) => (r, decode(n).trim().to_string()),
        None => (rest, String::new()),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, q),
        None => (rest, ""),
    };
    // Find the userinfo before cutting off the path: keys and passwords often contain a
    // raw '/' (standard base64), which must not end the authority early.
    let (userinfo, hostpath) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, rest),
    };
    let hostport = hostpath.split('/').next().unwrap_or_default();
    let (user, pass) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((a, b)) => (Some(decode(a)), Some(decode(b))),
            None => (Some(decode(u)), None),
        },
        None => (None, None),
    };
    let (host, port_spec) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or_else(|| invalid(scheme, "unclosed IPv6 bracket"))?;
        (h.to_string(), after.strip_prefix(':').unwrap_or_default().to_string())
    } else {
        match hostport.split_once(':') {
            Some((h, p)) => (h.to_string(), p.to_string()),
            None => (hostport.to_string(), String::new()),
        }
    };
    if host.is_empty() {
        return Err(invalid(scheme, "missing server address"));
    }
    let mut map = HashMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(decode(k).to_ascii_lowercase(), decode(v));
    }
    Ok(Uri { user, pass, host, port_spec, query: map, name })
}

/// Decode standard or URL-safe base64, with or without padding, ignoring whitespace.
/// What a `kova://` link asks to import: a subscription address or a share link.
/// Subscription pages build these like other clients' links: `kova://import/<address>`
/// (Remnawave and others append the address as it is), `kova://import?url=<address>` or
/// Clash's `kova://install-config?url=<address>`. `None` for anything else.
pub fn from_app_link(link: &str) -> Option<String> {
    let rest = link.trim().strip_prefix("kova://")?;
    let payload = if let Some(p) = rest.strip_prefix("import/") {
        p.to_string()
    } else {
        let query = rest.strip_prefix("import?").or_else(|| rest.strip_prefix("install-config?"))?;
        let value = query.split('&').find_map(|kv| kv.strip_prefix("url="))?;
        decode(value)
    };
    // An address passed whole may still be percent-encoded.
    let payload = if payload.contains("://") { payload } else { decode(&payload) };
    let scheme = payload.split("://").next()?;
    let ok = !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '+') && payload.len() > scheme.len() + 3;
    (ok && payload.len() <= 8192 && !payload.starts_with("kova://")).then_some(payload)
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let trimmed = clean.trim_end_matches('=');
    [&STANDARD_NO_PAD, &URL_SAFE_NO_PAD]
        .iter()
        .find_map(|e| e.decode(trimmed).ok())
        .or_else(|| STANDARD.decode(&clean).ok())
        .or_else(|| URL_SAFE.decode(&clean).ok())
}

fn b64_text(s: &str) -> Option<String> {
    b64_decode(s).and_then(|b| String::from_utf8(b).ok())
}

fn csv(s: Option<String>) -> Vec<String> {
    s.map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

fn transport_from(u: &Uri) -> Transport {
    let path = u.q(&["path"]).unwrap_or_default();
    let host = u.q(&["host"]);
    match u.q(&["type", "network"]).as_deref().unwrap_or("tcp") {
        "ws" | "websocket" => Transport::Ws { path: or_root(path), host },
        "grpc" | "gun" => Transport::Grpc {
            service_name: u.q(&["serviceName", "service_name"]).unwrap_or(path),
        },
        "http" | "h2" => Transport::H2 { path: or_root(path), host },
        "httpupgrade" => Transport::HttpUpgrade { path: or_root(path), host },
        "xhttp" | "splithttp" => Transport::Xhttp {
            path: or_root(path),
            host,
            mode: u.q(&["mode"]),
            extra: u.q(&["extra"]).and_then(|e| serde_json::from_str(&e).ok()),
        },
        _ => Transport::Tcp,
    }
}

fn or_root(p: String) -> String {
    if p.is_empty() { "/".into() } else { p }
}

fn tls_from(u: &Uri, default_on: bool) -> Option<Tls> {
    let security = u.q(&["security"]).unwrap_or_else(|| if default_on { "tls".into() } else { "none".into() });
    let reality = match security.as_str() {
        "reality" => Some(Reality {
            public_key: u.q(&["pbk", "publicKey", "public-key"]).unwrap_or_default(),
            short_id: u.q(&["sid", "shortId", "short-id"]),
            spider_x: u.q(&["spx", "spiderX"]),
        }),
        "tls" | "xtls" => None,
        _ => return None,
    };
    Some(Tls {
        sni: u.q(&["sni", "peer", "serverName"]),
        alpn: csv(u.q(&["alpn"])),
        fingerprint: u.q(&["fp", "fingerprint"]),
        insecure: u.flag(&["allowInsecure", "insecure", "allow_insecure", "skip-cert-verify"]),
        reality,
    })
}

// ---------------------------------------------------------------------------------------
// Per-protocol parsers

fn parse_vless(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "VLESS")?;
    let uuid = u.user.clone().filter(|x| !x.is_empty()).ok_or_else(|| invalid("VLESS", "missing UUID"))?;
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("VLESS")?,
        protocol: Protocol::Vless {
            uuid,
            flow: u.q(&["flow"]),
            encryption: u.q(&["encryption"]).filter(|e| e != "none"),
        },
        transport: transport_from(&u),
        tls: tls_from(&u, false),
        udp: true,
    })
}

fn parse_trojan(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "Trojan")?;
    let password = u.secret().ok_or_else(|| invalid("Trojan", "missing password"))?;
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("Trojan")?,
        protocol: Protocol::Trojan { password },
        transport: transport_from(&u),
        tls: tls_from(&u, true),
        udp: true,
    })
}

fn parse_vmess(s: &str) -> Result<Node, LinkError> {
    let body = &s["vmess://".len()..];
    let Some(json) = b64_text(body.split('#').next().unwrap_or(body)) else {
        // Some clients emit the URI form instead of base64 JSON.
        let u = split_uri(s, "VMess")?;
        return Ok(Node {
            name: u.name.clone(),
            server: u.host.clone(),
            port: u.port("VMess")?,
            protocol: Protocol::Vmess {
                uuid: u.user.clone().ok_or_else(|| invalid("VMess", "missing UUID"))?,
                alter_id: 0,
                cipher: u.q(&["encryption", "scy"]).unwrap_or_else(|| "auto".into()),
            },
            transport: transport_from(&u),
            tls: tls_from(&u, false),
            udp: true,
        });
    };
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| invalid("VMess", e.to_string()))?;
    let str_of = |k: &str| -> Option<String> {
        match v.get(k)? {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let server = str_of("add").ok_or_else(|| invalid("VMess", "missing address"))?;
    let port = str_of("port").and_then(|p| p.parse().ok()).ok_or_else(|| invalid("VMess", "missing port"))?;
    let path = str_of("path").unwrap_or_default();
    let host = str_of("host");
    let transport = match str_of("net").as_deref().unwrap_or("tcp") {
        "ws" => Transport::Ws { path: or_root(path), host },
        "grpc" => Transport::Grpc { service_name: path },
        "h2" | "http" => Transport::H2 { path: or_root(path), host },
        "httpupgrade" => Transport::HttpUpgrade { path: or_root(path), host },
        "xhttp" | "splithttp" => Transport::Xhttp { path: or_root(path), host, mode: None, extra: None },
        _ => Transport::Tcp,
    };
    let tls = match str_of("tls").as_deref() {
        Some("tls") => Some(Tls {
            sni: str_of("sni"),
            alpn: csv(str_of("alpn")),
            fingerprint: str_of("fp"),
            insecure: matches!(str_of("allowInsecure").as_deref(), Some("1" | "true")),
            reality: None,
        }),
        _ => None,
    };
    Ok(Node {
        name: str_of("ps").unwrap_or_default(),
        server,
        port,
        protocol: Protocol::Vmess {
            uuid: str_of("id").ok_or_else(|| invalid("VMess", "missing UUID"))?,
            alter_id: str_of("aid").and_then(|a| a.parse().ok()).unwrap_or(0),
            cipher: str_of("scy").unwrap_or_else(|| "auto".into()),
        },
        transport,
        tls,
        udp: true,
    })
}

fn parse_ss(s: &str) -> Result<Node, LinkError> {
    let body = &s["ss://".len()..];
    let (body, name) = match body.split_once('#') {
        Some((b, n)) => (b, decode(n).trim().to_string()),
        None => (body, String::new()),
    };
    let (body, query) = match body.split_once('?') {
        Some((b, q)) => (b.trim_end_matches('/'), q),
        None => (body.trim_end_matches('/'), ""),
    };
    // Legacy form: the whole `method:password@host:port` is base64.
    let full = if body.contains('@') {
        body.to_string()
    } else {
        b64_text(body).ok_or_else(|| invalid("Shadowsocks", "could not decode"))?
    };
    let (userinfo, hostport) = full.rsplit_once('@').ok_or_else(|| invalid("Shadowsocks", "missing server"))?;
    // SIP002 base64-encodes `method:password`; SS-2022 links percent-encode it instead.
    // Base64 never contains ':', so its presence tells the two apart.
    let creds = {
        let plain = decode(userinfo);
        if plain.contains(':') {
            plain
        } else {
            b64_text(userinfo).ok_or_else(|| invalid("Shadowsocks", "bad credentials"))?
        }
    };
    let (method, password) = creds.split_once(':').ok_or_else(|| invalid("Shadowsocks", "bad credentials"))?;
    let fake = format!("ss://x@{hostport}?{query}");
    let u = split_uri(&fake, "Shadowsocks")?;
    let plugin = u.q(&["plugin"]).and_then(|p| parse_sip003_plugin(&p));
    Ok(Node {
        name,
        server: u.host.clone(),
        port: u.port("Shadowsocks")?,
        protocol: Protocol::Shadowsocks { method: method.to_string(), password: password.to_string(), plugin },
        transport: Transport::Tcp,
        tls: None,
        udp: true,
    })
}

fn parse_sip003_plugin(spec: &str) -> Option<SsPlugin> {
    let mut parts = spec.split(';');
    let name = parts.next()?.trim();
    let mut kv: HashMap<String, String> = HashMap::new();
    let mut flags = Vec::new();
    for p in parts {
        match p.split_once('=') {
            Some((k, v)) => {
                kv.insert(k.to_string(), v.to_string());
            }
            None => flags.push(p.to_string()),
        }
    }
    let mut opts = indexmap::IndexMap::new();
    let plugin_name = match name {
        "obfs-local" | "simple-obfs" | "obfs" => {
            opts.insert("mode".into(), kv.get("obfs").cloned().unwrap_or_else(|| "http".into()).into());
            if let Some(h) = kv.get("obfs-host") {
                opts.insert("host".into(), h.clone().into());
            }
            "obfs"
        }
        "v2ray-plugin" => {
            opts.insert("mode".into(), kv.get("mode").cloned().unwrap_or_else(|| "websocket".into()).into());
            if let Some(h) = kv.get("host") {
                opts.insert("host".into(), h.clone().into());
            }
            if let Some(p) = kv.get("path") {
                opts.insert("path".into(), p.clone().into());
            }
            if flags.iter().any(|f| f == "tls") || kv.contains_key("tls") {
                opts.insert("tls".into(), true.into());
            }
            "v2ray-plugin"
        }
        other => {
            for (k, v) in kv {
                opts.insert(k, v.into());
            }
            other
        }
    };
    Some(SsPlugin { name: plugin_name.to_string(), opts })
}

fn parse_hy2(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "Hysteria2")?;
    let password = u.secret().ok_or_else(|| invalid("Hysteria2", "missing password"))?;
    // `host:443,20000-30000` or `?mport=20000-30000` means port hopping.
    let hop = u.port_spec.split_once(',').map(|(_, r)| r.to_string()).or_else(|| u.q(&["mport", "ports"]));
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("Hysteria2")?,
        protocol: Protocol::Hysteria2 {
            password,
            obfs: u.q(&["obfs"]).filter(|o| o != "none"),
            obfs_password: u.q(&["obfs-password", "obfs_password", "obfsParam"]),
            ports: hop,
            up_mbps: u.q(&["upmbps", "up"]).and_then(|v| v.trim_end_matches(" Mbps").parse().ok()),
            down_mbps: u.q(&["downmbps", "down"]).and_then(|v| v.trim_end_matches(" Mbps").parse().ok()),
        },
        transport: Transport::Tcp,
        tls: Some(Tls {
            sni: u.q(&["sni", "peer"]),
            alpn: csv(u.q(&["alpn"])),
            fingerprint: None,
            insecure: u.flag(&["insecure", "allowInsecure"]),
            reality: None,
        }),
        udp: true,
    })
}

fn parse_tuic(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "TUIC")?;
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("TUIC")?,
        protocol: Protocol::Tuic {
            uuid: u.user.clone().ok_or_else(|| invalid("TUIC", "missing UUID"))?,
            password: u.pass.clone().unwrap_or_default(),
            congestion_control: u.q(&["congestion_control", "congestion-control", "congestion_controller"]),
            udp_relay_mode: u.q(&["udp_relay_mode", "udp-relay-mode"]),
        },
        transport: Transport::Tcp,
        tls: Some(Tls {
            sni: u.q(&["sni", "peer"]),
            alpn: {
                let a = csv(u.q(&["alpn"]));
                if a.is_empty() { vec!["h3".into()] } else { a }
            },
            fingerprint: None,
            insecure: u.flag(&["allow_insecure", "insecure", "allowInsecure"]),
            reality: None,
        }),
        udp: true,
    })
}

fn parse_wireguard(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "WireGuard")?;
    let private_key = u.user.clone().or_else(|| u.q(&["privatekey", "private_key", "secretkey"]));
    let private_key = private_key.ok_or_else(|| invalid("WireGuard", "missing private key"))?;
    let public_key = u
        .q(&["publickey", "public_key", "peer_public_key", "pbk"])
        .ok_or_else(|| invalid("WireGuard", "missing peer public key"))?;
    let addresses = csv(u.q(&["address", "ip", "addresses", "local_address"]))
        .into_iter()
        .map(|a| if a.contains('/') { a } else if a.contains(':') { format!("{a}/128") } else { format!("{a}/32") })
        .collect();
    let reserved = u
        .q(&["reserved"])
        .map(|r| {
            let nums: Vec<u8> = r.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if nums.len() == 3 { nums } else { b64_decode(&r).filter(|b| b.len() == 3).unwrap_or_default() }
        })
        .unwrap_or_default();
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("WireGuard")?,
        protocol: Protocol::Wireguard {
            private_key,
            public_key,
            pre_shared_key: u.q(&["presharedkey", "pre_shared_key", "psk"]),
            addresses,
            mtu: u.q(&["mtu"]).and_then(|m| m.parse().ok()),
            reserved,
        },
        transport: Transport::Tcp,
        tls: None,
        udp: true,
    })
}

fn parse_anytls(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "AnyTLS")?;
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: u.port("AnyTLS")?,
        protocol: Protocol::Anytls { password: u.secret().ok_or_else(|| invalid("AnyTLS", "missing password"))? },
        transport: Transport::Tcp,
        tls: Some(Tls {
            sni: u.q(&["sni", "peer"]),
            alpn: csv(u.q(&["alpn"])),
            fingerprint: u.q(&["fp"]),
            insecure: u.flag(&["insecure", "allowInsecure"]),
            reality: None,
        }),
        udp: true,
    })
}

fn parse_socks(s: &str) -> Result<Node, LinkError> {
    let u = split_uri(s, "SOCKS")?;
    let (username, password) = match (&u.user, &u.pass) {
        (Some(a), Some(b)) => (Some(a.clone()), Some(b.clone())),
        (Some(a), None) => match b64_text(a).as_deref().and_then(|t| t.split_once(':')) {
            Some((x, y)) => (Some(x.to_string()), Some(y.to_string())),
            None => (Some(a.clone()), None),
        },
        _ => (None, None),
    };
    Ok(Node {
        name: u.name.clone(),
        server: u.host.clone(),
        port: if u.port_spec.is_empty() { 1080 } else { u.port("SOCKS")? },
        protocol: Protocol::Socks { username, password },
        transport: Transport::Tcp,
        tls: None,
        udp: true,
    })
}

// ---------------------------------------------------------------------------------------
// Export

const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

fn host_for_uri(h: &str) -> String {
    if h.contains(':') { format!("[{h}]") } else { h.to_string() }
}

struct Query(Vec<(String, String)>);
impl Query {
    fn push(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        if !v.is_empty() {
            self.0.push((k.into(), v));
        }
    }
    fn render(&self) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        let joined: Vec<String> = self.0.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect();
        format!("?{}", joined.join("&"))
    }
}

fn push_stream(q: &mut Query, n: &Node) {
    match &n.transport {
        Transport::Tcp => q.push("type", "tcp"),
        Transport::Ws { path, host } => {
            q.push("type", "ws");
            q.push("path", path.clone());
            q.push("host", host.clone().unwrap_or_default());
        }
        Transport::Grpc { service_name } => {
            q.push("type", "grpc");
            q.push("serviceName", service_name.clone());
        }
        Transport::H2 { path, host } => {
            q.push("type", "http");
            q.push("path", path.clone());
            q.push("host", host.clone().unwrap_or_default());
        }
        Transport::HttpUpgrade { path, host } => {
            q.push("type", "httpupgrade");
            q.push("path", path.clone());
            q.push("host", host.clone().unwrap_or_default());
        }
        Transport::Xhttp { path, host, mode, extra } => {
            q.push("type", "xhttp");
            q.push("path", path.clone());
            q.push("host", host.clone().unwrap_or_default());
            q.push("mode", mode.clone().unwrap_or_default());
            if let Some(e) = extra {
                q.push("extra", e.to_string());
            }
        }
    }
    match &n.tls {
        None => q.push("security", "none"),
        Some(t) => {
            q.push("security", if t.reality.is_some() { "reality" } else { "tls" });
            q.push("sni", t.sni.clone().unwrap_or_default());
            q.push("alpn", t.alpn.join(","));
            q.push("fp", t.fingerprint.clone().unwrap_or_default());
            if t.insecure {
                q.push("allowInsecure", "1");
            }
            if let Some(r) = &t.reality {
                q.push("pbk", r.public_key.clone());
                q.push("sid", r.short_id.clone().unwrap_or_default());
                q.push("spx", r.spider_x.clone().unwrap_or_default());
            }
        }
    }
}

/// Render a node back into a share link. `None` for types that have no link format.
pub fn to_link(n: &Node) -> Option<String> {
    let host = host_for_uri(&n.server);
    let frag = format!("#{}", enc(&n.name));
    let mut q = Query(Vec::new());
    let tls_simple = |q: &mut Query| {
        if let Some(t) = &n.tls {
            q.push("sni", t.sni.clone().unwrap_or_default());
            q.push("alpn", t.alpn.join(","));
            if t.insecure {
                q.push("insecure", "1");
            }
        }
    };
    let link = match &n.protocol {
        Protocol::Vless { uuid, flow, encryption } => {
            q.push("encryption", encryption.clone().unwrap_or_else(|| "none".into()));
            q.push("flow", flow.clone().unwrap_or_default());
            push_stream(&mut q, n);
            format!("vless://{}@{host}:{}{}", enc(uuid), n.port, q.render())
        }
        Protocol::Trojan { password } => {
            push_stream(&mut q, n);
            format!("trojan://{}@{host}:{}{}", enc(password), n.port, q.render())
        }
        Protocol::Vmess { uuid, alter_id, cipher } => {
            let (net, path, hosth) = match &n.transport {
                Transport::Tcp => ("tcp", String::new(), None),
                Transport::Ws { path, host } => ("ws", path.clone(), host.clone()),
                Transport::Grpc { service_name } => ("grpc", service_name.clone(), None),
                Transport::H2 { path, host } => ("h2", path.clone(), host.clone()),
                Transport::HttpUpgrade { path, host } => ("httpupgrade", path.clone(), host.clone()),
                Transport::Xhttp { path, host, .. } => ("xhttp", path.clone(), host.clone()),
            };
            let t = n.tls.clone().unwrap_or_default();
            let j = serde_json::json!({
                "v": "2", "ps": n.name, "add": n.server, "port": n.port.to_string(), "id": uuid,
                "aid": alter_id.to_string(), "scy": cipher, "net": net, "type": "none",
                "host": hosth.unwrap_or_default(), "path": path,
                "tls": if n.tls.is_some() { "tls" } else { "" },
                "sni": t.sni.unwrap_or_default(), "alpn": t.alpn.join(","), "fp": t.fingerprint.unwrap_or_default(),
            });
            let b = base64::engine::general_purpose::STANDARD.encode(j.to_string());
            return Some(format!("vmess://{b}"));
        }
        Protocol::Shadowsocks { method, password, plugin } => {
            let creds = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{method}:{password}"));
            if let Some(p) = plugin {
                let mut spec = match p.name.as_str() {
                    "obfs" => "obfs-local".to_string(),
                    other => other.to_string(),
                };
                for (k, v) in &p.opts {
                    let key = match (p.name.as_str(), k.as_str()) {
                        ("obfs", "mode") => "obfs",
                        ("obfs", "host") => "obfs-host",
                        (_, k) => k,
                    };
                    match v {
                        serde_json::Value::Bool(true) => spec.push_str(&format!(";{key}")),
                        serde_json::Value::String(s) => spec.push_str(&format!(";{key}={s}")),
                        other => spec.push_str(&format!(";{key}={other}")),
                    }
                }
                q.push("plugin", spec);
            }
            format!("ss://{creds}@{host}:{}{}", n.port, q.render())
        }
        Protocol::Hysteria2 { password, obfs, obfs_password, ports, .. } => {
            tls_simple(&mut q);
            q.push("obfs", obfs.clone().unwrap_or_default());
            q.push("obfs-password", obfs_password.clone().unwrap_or_default());
            q.push("mport", ports.clone().unwrap_or_default());
            format!("hysteria2://{}@{host}:{}/{}", enc(password), n.port, q.render())
        }
        Protocol::Tuic { uuid, password, congestion_control, udp_relay_mode } => {
            tls_simple(&mut q);
            q.push("congestion_control", congestion_control.clone().unwrap_or_default());
            q.push("udp_relay_mode", udp_relay_mode.clone().unwrap_or_default());
            format!("tuic://{}:{}@{host}:{}{}", enc(uuid), enc(password), n.port, q.render())
        }
        Protocol::Anytls { password } => {
            tls_simple(&mut q);
            format!("anytls://{}@{host}:{}/{}", enc(password), n.port, q.render())
        }
        Protocol::Wireguard { private_key, public_key, pre_shared_key, addresses, mtu, reserved } => {
            q.push("publickey", public_key.clone());
            q.push("presharedkey", pre_shared_key.clone().unwrap_or_default());
            q.push("address", addresses.join(","));
            q.push("mtu", mtu.map(|m| m.to_string()).unwrap_or_default());
            if reserved.len() == 3 {
                q.push("reserved", reserved.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(","));
            }
            format!("wireguard://{}@{host}:{}{}", enc(private_key), n.port, q.render())
        }
        Protocol::Socks { username, password } => match (username, password) {
            (Some(u), Some(p)) => {
                let b = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
                format!("socks://{b}@{host}:{}", n.port)
            }
            _ => format!("socks://{host}:{}", n.port),
        },
        Protocol::Http { .. } | Protocol::Other { .. } => return None,
    };
    Some(link + &frag)
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_links() {
        let sub = "https://sub.example.com/abc?x=1#Name";
        assert_eq!(from_app_link(&format!("kova://import/{sub}")).as_deref(), Some(sub));
        assert_eq!(from_app_link("kova://import/https%3A%2F%2Fsub.example.com%2Fabc").as_deref(), Some("https://sub.example.com/abc"));
        assert_eq!(from_app_link("kova://import?url=https%3A%2F%2Fsub.example.com%2Fa%3Fb%3D1").as_deref(), Some("https://sub.example.com/a?b=1"));
        assert_eq!(from_app_link("kova://install-config?url=https%3A%2F%2Fs.example%2Fc&name=x").as_deref(), Some("https://s.example/c"));
        assert_eq!(from_app_link("kova://import/vless://id@203.0.113.1:443#A").as_deref(), Some("vless://id@203.0.113.1:443#A"));
        for bad in ["kova://connect", "kova://import/", "kova://import/notalink", "happ://add/https://x", "kova://import/kova://import/https://x", "kova://import?name=x"] {
            assert_eq!(from_app_link(bad), None, "{bad}");
        }
    }

    use super::*;

    #[test]
    fn vless_reality_vision() {
        let n = parse_link("vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.42:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw&sid=6ba85179&type=tcp#NL%20Amsterdam").unwrap();
        assert_eq!(n.name, "NL Amsterdam");
        assert_eq!(n.port, 443);
        assert!(matches!(&n.protocol, Protocol::Vless { flow: Some(f), encryption: None, .. } if f == "xtls-rprx-vision"));
        let t = n.tls.as_ref().unwrap();
        assert_eq!(t.sni.as_deref(), Some("www.microsoft.com"));
        assert_eq!(t.reality.as_ref().unwrap().short_id.as_deref(), Some("6ba85179"));
        assert_eq!(n.transport_label(), "Reality · Vision");
    }

    #[test]
    fn vless_xhttp_and_ipv6() {
        let n = parse_link("vless://id@[2001:db8::1]:8443?security=tls&type=xhttp&path=%2Fapi&mode=packet-up&host=cdn.example.com#X").unwrap();
        assert_eq!(n.server, "2001:db8::1");
        assert_eq!(n.port, 8443);
        assert!(matches!(&n.transport, Transport::Xhttp { path, mode: Some(m), .. } if path == "/api" && m == "packet-up"));
    }

    #[test]
    fn vmess_base64_json() {
        let j = r#"{"v":"2","ps":"US New York","add":"192.0.2.140","port":443,"id":"a3482e88-686a-4a58-8126-99c9df64b7bf","aid":"0","scy":"auto","net":"ws","type":"none","host":"cdn.example.com","path":"/ray","tls":"tls","sni":"cdn.example.com"}"#;
        let link = format!("vmess://{}", base64::engine::general_purpose::STANDARD.encode(j));
        let n = parse_link(&link).unwrap();
        assert_eq!(n.name, "US New York");
        assert_eq!(n.port, 443);
        assert!(matches!(&n.transport, Transport::Ws { path, host: Some(h) } if path == "/ray" && h == "cdn.example.com"));
        assert!(n.tls.is_some());
    }

    #[test]
    fn ss_sip002_and_legacy_and_2022() {
        let a = parse_link("ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@198.51.100.1:8388#A").unwrap();
        assert!(matches!(&a.protocol, Protocol::Shadowsocks { method, password, .. } if method == "aes-256-gcm" && password == "password"));
        let legacy = format!("ss://{}#B", base64::engine::general_purpose::STANDARD.encode("chacha20-ietf-poly1305:pw@198.51.100.2:443"));
        let b = parse_link(&legacy).unwrap();
        assert_eq!(b.server, "198.51.100.2");
        assert_eq!(b.port, 443);
        let c = parse_link("ss://2022-blake3-aes-256-gcm:YctPZ6U7xPPcU%2Bgp3u%2BCjb0V5IOt%2FY1yR4EJSzvTxPQ%3D@198.51.100.3:443#C").unwrap();
        assert!(matches!(&c.protocol, Protocol::Shadowsocks { method, password, .. } if method == "2022-blake3-aes-256-gcm" && password.ends_with('=')));
        let d = parse_link("ss://YWVzLTEyOC1nY206dGVzdA@192.0.2.1:8888/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dexample.com#D").unwrap();
        assert!(matches!(&d.protocol, Protocol::Shadowsocks { plugin: Some(p), .. } if p.name == "obfs" && p.opts["host"] == "example.com"));
    }

    #[test]
    fn hysteria2_port_hopping() {
        let n = parse_link("hy2://secret@203.0.113.87:443,20000-30000/?sni=example.com&obfs=salamander&obfs-password=pw&insecure=1#DE").unwrap();
        assert_eq!(n.port, 443);
        assert!(matches!(&n.protocol, Protocol::Hysteria2 { ports: Some(p), obfs: Some(o), .. } if p == "20000-30000" && o == "salamander"));
        assert!(n.tls.unwrap().insecure);
    }

    #[test]
    fn tuic_and_anytls_and_wireguard() {
        let t = parse_link("tuic://uuid:pass@198.51.100.120:443?congestion_control=bbr&alpn=h3&sni=a.example#SE").unwrap();
        assert!(matches!(&t.protocol, Protocol::Tuic { password, .. } if password == "pass"));
        let a = parse_link("anytls://pw@192.0.2.71:443/?sni=b.example&insecure=1#TR").unwrap();
        assert!(matches!(&a.protocol, Protocol::Anytls { .. }));
        let w = parse_link("wireguard://cHJpdmF0ZQ%3D%3D@198.51.100.14:51820?publickey=cHVibGlj&address=10.0.0.2,fd00::2&mtu=1408&reserved=1,2,3#PL").unwrap();
        match &w.protocol {
            Protocol::Wireguard { private_key, addresses, reserved, mtu, .. } => {
                assert_eq!(private_key, "cHJpdmF0ZQ==");
                assert_eq!(addresses, &["10.0.0.2/32", "fd00::2/128"]);
                assert_eq!(reserved, &[1, 2, 3]);
                assert_eq!(*mtu, Some(1408));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn roundtrip_through_links() {
        let links = [
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.42:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=KEY&sid=6ba85179&type=tcp#NL",
            "trojan://pw@192.0.2.33:443?security=tls&sni=t.example&type=grpc&serviceName=svc#GB",
            "hysteria2://secret@203.0.113.87:443/?sni=example.com&obfs=salamander&obfs-password=pw#DE",
            "tuic://uuid:pass@198.51.100.120:443?congestion_control=bbr&sni=a.example#SE",
            "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@198.51.100.1:8388#SS",
        ];
        for l in links {
            let a = parse_link(l).unwrap();
            let b = parse_link(&to_link(&a).unwrap()).unwrap();
            assert_eq!(a, b, "roundtrip failed for {l}");
        }
        let vm = parse_link(&to_link(&parse_link(&format!("vmess://{}", base64::engine::general_purpose::STANDARD.encode(r#"{"ps":"x","add":"a.example","port":"80","id":"u","net":"ws","path":"/p"}"#))).unwrap()).unwrap()).unwrap();
        assert_eq!(vm.server, "a.example");
    }

    #[test]
    fn userinfo_with_slash_and_colon() {
        // Raw standard-base64 key with '/' and '+', not percent-encoded.
        let w = parse_link("wireguard://aB3/xY+z9Q=@198.51.100.14:51820?publickey=cHVi&address=10.0.0.2#WG").unwrap();
        assert_eq!(w.server, "198.51.100.14");
        assert_eq!(w.port, 51820);
        assert!(matches!(&w.protocol, Protocol::Wireguard { private_key, .. } if private_key == "aB3/xY+z9Q="));
        let t = parse_link("trojan://pa:ss/word@192.0.2.33:443?sni=t.example#T").unwrap();
        assert!(matches!(&t.protocol, Protocol::Trojan { password } if password == "pa:ss/word"));
        assert_eq!(t.server, "192.0.2.33");
        let a = parse_link("anytls://x:y@192.0.2.71:443/?sni=b.example#A").unwrap();
        assert!(matches!(&a.protocol, Protocol::Anytls { password } if password == "x:y"));
        // A path after the host still works.
        let v = parse_link("vless://id@a.example:443/?security=tls#V").unwrap();
        assert_eq!(v.port, 443);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_link("hello"), Err(LinkError::NotALink));
        assert!(matches!(parse_link("ssr://abc"), Err(LinkError::Scheme(_))));
        assert!(parse_link("vless://@:443").is_err());
    }
}
