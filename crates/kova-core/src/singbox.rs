//! sing-box JSON `outbounds` -> [`Node`]. Group and utility outbounds are skipped.

use serde_json::Value;

use crate::model::*;

fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

const SKIP: &[&str] = &["selector", "urltest", "direct", "block", "dns"];

/// Returns the nodes and a warning per outbound that could not be converted.
pub fn parse_outbounds(doc: &Value) -> (Vec<Node>, Vec<String>) {
    let mut nodes = Vec::new();
    let mut warnings = Vec::new();
    for o in doc.get("outbounds").and_then(Value::as_array).into_iter().flatten() {
        let kind = s(o, "type").unwrap_or_default();
        if SKIP.contains(&kind.as_str()) {
            continue;
        }
        match convert(o, &kind) {
            Ok(n) => nodes.push(n),
            Err(e) => warnings.push(e),
        }
    }
    (nodes, warnings)
}

fn tls(o: &Value, always: bool) -> Option<Tls> {
    let t = o.get("tls");
    let enabled = t.and_then(|t| t.get("enabled")).and_then(Value::as_bool).unwrap_or(false);
    if !(always || enabled) {
        return None;
    }
    let t = t.cloned().unwrap_or(Value::Null);
    let reality = t.get("reality").filter(|r| r.get("enabled").and_then(Value::as_bool).unwrap_or(false)).map(|r| Reality {
        public_key: s(r, "public_key").unwrap_or_default(),
        short_id: s(r, "short_id"),
        spider_x: None,
    });
    Some(Tls {
        sni: s(&t, "server_name"),
        alpn: t.get("alpn").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        fingerprint: t.get("utls").and_then(|u| s(u, "fingerprint")),
        insecure: t.get("insecure").and_then(Value::as_bool).unwrap_or(false),
        reality,
    })
}

fn transport(o: &Value) -> Transport {
    let Some(t) = o.get("transport") else { return Transport::Tcp };
    let host = t.get("headers").and_then(|h| s(h, "Host")).or_else(|| s(t, "host"));
    let path = s(t, "path").unwrap_or_else(|| "/".into());
    match s(t, "type").as_deref() {
        Some("ws") => Transport::Ws { path, host },
        Some("grpc") => Transport::Grpc { service_name: s(t, "service_name").unwrap_or_default() },
        Some("http") => Transport::H2 { path, host: t.get("host").and_then(Value::as_array).and_then(|a| a.first()).and_then(|h| h.as_str().map(String::from)).or(host) },
        Some("httpupgrade") => Transport::HttpUpgrade { path, host },
        _ => Transport::Tcp,
    }
}

fn convert(o: &Value, kind: &str) -> Result<Node, String> {
    let name = s(o, "tag").unwrap_or_else(|| kind.to_string());
    let need = |k: &str| s(o, k).ok_or_else(|| format!("{name}: missing `{k}`"));
    let server = need("server")?;
    let port: u16 = s(o, "server_port").and_then(|p| p.parse().ok()).ok_or_else(|| format!("{name}: missing port"))?;
    let (protocol, tr, tl) = match kind {
        "vless" => (Protocol::Vless { uuid: need("uuid")?, flow: s(o, "flow"), encryption: None }, transport(o), tls(o, false)),
        "vmess" => (
            Protocol::Vmess {
                uuid: need("uuid")?,
                alter_id: s(o, "alter_id").and_then(|a| a.parse().ok()).unwrap_or(0),
                cipher: s(o, "security").unwrap_or_else(|| "auto".into()),
            },
            transport(o),
            tls(o, false),
        ),
        "trojan" => (Protocol::Trojan { password: need("password")? }, transport(o), tls(o, false)),
        "shadowsocks" => (
            Protocol::Shadowsocks { method: need("method")?, password: need("password")?, plugin: None },
            Transport::Tcp,
            None,
        ),
        "hysteria2" => (
            Protocol::Hysteria2 {
                password: need("password")?,
                obfs: o.get("obfs").and_then(|x| s(x, "type")),
                obfs_password: o.get("obfs").and_then(|x| s(x, "password")),
                ports: o.get("server_ports").and_then(Value::as_array).and_then(|a| a.first()).and_then(|p| p.as_str()).map(|p| p.replace(':', "-")),
                up_mbps: s(o, "up_mbps").and_then(|x| x.parse().ok()),
                down_mbps: s(o, "down_mbps").and_then(|x| x.parse().ok()),
            },
            Transport::Tcp,
            tls(o, true),
        ),
        "tuic" => (
            Protocol::Tuic {
                uuid: need("uuid")?,
                password: s(o, "password").unwrap_or_default(),
                congestion_control: s(o, "congestion_control"),
                udp_relay_mode: s(o, "udp_relay_mode"),
            },
            Transport::Tcp,
            tls(o, true),
        ),
        "anytls" => (Protocol::Anytls { password: need("password")? }, Transport::Tcp, tls(o, true)),
        "wireguard" => (
            Protocol::Wireguard {
                private_key: need("private_key")?,
                public_key: need("peer_public_key")?,
                pre_shared_key: s(o, "pre_shared_key"),
                addresses: o.get("local_address").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                mtu: s(o, "mtu").and_then(|m| m.parse().ok()),
                reserved: o.get("reserved").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_u64().map(|v| v as u8)).collect()).unwrap_or_default(),
            },
            Transport::Tcp,
            None,
        ),
        "socks" => (Protocol::Socks { username: s(o, "username"), password: s(o, "password") }, Transport::Tcp, None),
        "http" => (Protocol::Http { username: s(o, "username"), password: s(o, "password") }, Transport::Tcp, tls(o, false)),
        other => return Err(format!("{name}: sing-box type `{other}` is not supported")),
    };
    Ok(Node { name, server, port, protocol, transport: tr, tls: tl, udp: true })
}
