//! Clash / Mihomo `proxies:` entries <-> [`Node`].

use serde_json::{Map, Value, json};

use crate::model::*;

fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn b(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(x)) => *x,
        Some(Value::String(x)) => x == "true",
        _ => false,
    }
}

fn list(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Some(Value::String(x)) => x.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn tls_of(p: &Value, always: bool) -> Option<Tls> {
    let reality = p.get("reality-opts").map(|r| Reality {
        public_key: s(r, "public-key").unwrap_or_default(),
        short_id: s(r, "short-id"),
        spider_x: None,
    });
    if !(always || b(p, "tls") || reality.is_some()) {
        return None;
    }
    Some(Tls {
        sni: s(p, "servername").or_else(|| s(p, "sni")),
        alpn: list(p, "alpn"),
        fingerprint: s(p, "client-fingerprint"),
        insecure: b(p, "skip-cert-verify"),
        reality,
        pin: s(p, "fingerprint").and_then(|f| crate::links::normalise_pin(&f)),
    })
}

fn transport_of(p: &Value) -> Transport {
    match s(p, "network").as_deref() {
        Some("ws") => {
            let o = p.get("ws-opts").cloned().unwrap_or(Value::Null);
            let path = s(&o, "path").unwrap_or_else(|| "/".into());
            let host = o.get("headers").and_then(|h| s(h, "Host").or_else(|| s(h, "host")));
            if b(&o, "v2ray-http-upgrade") {
                Transport::HttpUpgrade { path, host }
            } else {
                Transport::Ws { path, host }
            }
        }
        Some("grpc") => Transport::Grpc {
            service_name: p.get("grpc-opts").and_then(|o| s(o, "grpc-service-name")).unwrap_or_default(),
        },
        Some("h2") => {
            let o = p.get("h2-opts").cloned().unwrap_or(Value::Null);
            Transport::H2 { path: s(&o, "path").unwrap_or_else(|| "/".into()), host: list(&o, "host").into_iter().next() }
        }
        Some("xhttp") => {
            let o = p.get("xhttp-opts").cloned().unwrap_or(Value::Null);
            Transport::Xhttp { path: s(&o, "path").unwrap_or_else(|| "/".into()), host: s(&o, "host"), mode: s(&o, "mode"), extra: None }
        }
        _ => Transport::Tcp,
    }
}

/// Convert one Clash proxy mapping. Unknown types become [`Protocol::Other`].
pub fn from_clash(p: &Value) -> Result<Node, String> {
    let name = s(p, "name").ok_or("proxy without a name")?;
    let kind = s(p, "type").ok_or_else(|| format!("{name}: missing type"))?.to_ascii_lowercase();
    let server = s(p, "server").unwrap_or_default();
    let port: u16 = s(p, "port").and_then(|x| x.parse().ok()).unwrap_or(0);
    let udp = p.get("udp").map(|_| b(p, "udp")).unwrap_or(true);
    let need = |k: &str| s(p, k).ok_or_else(|| format!("{name}: missing `{k}`"));
    let (protocol, transport, tls) = match kind.as_str() {
        "vless" => (
            Protocol::Vless { uuid: need("uuid")?, flow: s(p, "flow"), encryption: s(p, "encryption").filter(|e| e != "none") },
            transport_of(p),
            tls_of(p, false),
        ),
        "vmess" => (
            Protocol::Vmess {
                uuid: need("uuid")?,
                alter_id: s(p, "alterId").and_then(|a| a.parse().ok()).unwrap_or(0),
                cipher: s(p, "cipher").unwrap_or_else(|| "auto".into()),
            },
            transport_of(p),
            tls_of(p, false),
        ),
        "trojan" => (Protocol::Trojan { password: need("password")? }, transport_of(p), tls_of(p, true)),
        "ss" => {
            let plugin = s(p, "plugin").map(|n| SsPlugin {
                name: n,
                opts: p.get("plugin-opts").and_then(|o| o.as_object()).map(|m| m.clone().into_iter().collect()).unwrap_or_default(),
            });
            (Protocol::Shadowsocks { method: need("cipher")?, password: need("password")?, plugin }, Transport::Tcp, None)
        }
        "hysteria2" => (
            Protocol::Hysteria2 {
                password: need("password")?,
                obfs: s(p, "obfs"),
                obfs_password: s(p, "obfs-password"),
                ports: s(p, "ports"),
                up_mbps: s(p, "up").and_then(|x| x.split_whitespace().next().and_then(|n| n.parse().ok())),
                down_mbps: s(p, "down").and_then(|x| x.split_whitespace().next().and_then(|n| n.parse().ok())),
            },
            Transport::Tcp,
            tls_of(p, true),
        ),
        "tuic" => (
            Protocol::Tuic {
                uuid: need("uuid")?,
                password: s(p, "password").unwrap_or_default(),
                congestion_control: s(p, "congestion-controller"),
                udp_relay_mode: s(p, "udp-relay-mode"),
            },
            Transport::Tcp,
            tls_of(p, true),
        ),
        "wireguard" => {
            let mut addresses = Vec::new();
            if let Some(ip) = s(p, "ip") {
                addresses.push(if ip.contains('/') { ip } else { format!("{ip}/32") });
            }
            if let Some(ip) = s(p, "ipv6") {
                addresses.push(if ip.contains('/') { ip } else { format!("{ip}/128") });
            }
            let reserved = match p.get("reserved") {
                Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_u64().map(|v| v as u8)).collect(),
                Some(Value::String(r)) => crate::links::b64_decode(r).unwrap_or_default(),
                _ => Vec::new(),
            };
            (
                Protocol::Wireguard {
                    private_key: need("private-key")?,
                    public_key: need("public-key")?,
                    pre_shared_key: s(p, "pre-shared-key"),
                    addresses,
                    mtu: s(p, "mtu").and_then(|m| m.parse().ok()),
                    reserved,
                },
                Transport::Tcp,
                None,
            )
        }
        "anytls" => (Protocol::Anytls { password: need("password")? }, Transport::Tcp, tls_of(p, true)),
        "socks5" => (Protocol::Socks { username: s(p, "username"), password: s(p, "password") }, Transport::Tcp, tls_of(p, false)),
        "http" => (Protocol::Http { username: s(p, "username"), password: s(p, "password") }, Transport::Tcp, tls_of(p, false)),
        other => (Protocol::Other { kind: other.to_string(), raw: p.clone() }, Transport::Tcp, None),
    };
    Ok(Node { name, server, port, protocol, transport, tls, udp })
}

/// Render a node as a Mihomo `proxies:` entry.
pub fn to_clash(n: &Node) -> Value {
    if let Protocol::Other { raw, .. } = &n.protocol {
        let mut raw = raw.clone();
        raw["name"] = n.name.clone().into();
        return raw;
    }
    let mut m = Map::new();
    let put = |m: &mut Map<String, Value>, k: &str, v: Value| {
        m.insert(k.into(), v);
    };
    put(&mut m, "name", n.name.clone().into());
    let kind = match &n.protocol {
        Protocol::Vless { .. } => "vless",
        Protocol::Vmess { .. } => "vmess",
        Protocol::Trojan { .. } => "trojan",
        Protocol::Shadowsocks { .. } => "ss",
        Protocol::Hysteria2 { .. } => "hysteria2",
        Protocol::Tuic { .. } => "tuic",
        Protocol::Wireguard { .. } => "wireguard",
        Protocol::Anytls { .. } => "anytls",
        Protocol::Socks { .. } => "socks5",
        Protocol::Http { .. } => "http",
        Protocol::Other { .. } => unreachable!(),
    };
    put(&mut m, "type", kind.into());
    put(&mut m, "server", n.server.clone().into());
    put(&mut m, "port", n.port.into());
    put(&mut m, "udp", n.udp.into());

    match &n.protocol {
        Protocol::Vless { uuid, flow, encryption } => {
            put(&mut m, "uuid", uuid.clone().into());
            if let Some(f) = flow {
                put(&mut m, "flow", f.clone().into());
            }
            if let Some(e) = encryption {
                put(&mut m, "encryption", e.clone().into());
            }
            put(&mut m, "packet-encoding", "xudp".into());
        }
        Protocol::Vmess { uuid, alter_id, cipher } => {
            put(&mut m, "uuid", uuid.clone().into());
            put(&mut m, "alterId", (*alter_id).into());
            put(&mut m, "cipher", cipher.clone().into());
        }
        Protocol::Trojan { password } | Protocol::Anytls { password } => put(&mut m, "password", password.clone().into()),
        Protocol::Shadowsocks { method, password, plugin } => {
            put(&mut m, "cipher", method.clone().into());
            put(&mut m, "password", password.clone().into());
            if let Some(p) = plugin {
                put(&mut m, "plugin", p.name.clone().into());
                put(&mut m, "plugin-opts", Value::Object(p.opts.clone().into_iter().collect()));
            }
        }
        Protocol::Hysteria2 { password, obfs, obfs_password, ports, up_mbps, down_mbps } => {
            put(&mut m, "password", password.clone().into());
            if let Some(o) = obfs {
                put(&mut m, "obfs", o.clone().into());
                put(&mut m, "obfs-password", obfs_password.clone().unwrap_or_default().into());
            }
            if let Some(p) = ports {
                put(&mut m, "ports", p.clone().into());
            }
            if let Some(u) = up_mbps {
                put(&mut m, "up", format!("{u} Mbps").into());
            }
            if let Some(d) = down_mbps {
                put(&mut m, "down", format!("{d} Mbps").into());
            }
        }
        Protocol::Tuic { uuid, password, congestion_control, udp_relay_mode } => {
            put(&mut m, "uuid", uuid.clone().into());
            put(&mut m, "password", password.clone().into());
            if let Some(c) = congestion_control {
                put(&mut m, "congestion-controller", c.clone().into());
            }
            if let Some(u) = udp_relay_mode {
                put(&mut m, "udp-relay-mode", u.clone().into());
            }
        }
        Protocol::Wireguard { private_key, public_key, pre_shared_key, addresses, mtu, reserved } => {
            put(&mut m, "private-key", private_key.clone().into());
            put(&mut m, "public-key", public_key.clone().into());
            if let Some(p) = pre_shared_key {
                put(&mut m, "pre-shared-key", p.clone().into());
            }
            for a in addresses {
                let ip = a.split('/').next().unwrap_or(a).to_string();
                put(&mut m, if a.contains(':') { "ipv6" } else { "ip" }, ip.into());
            }
            if let Some(v) = mtu {
                put(&mut m, "mtu", (*v).into());
            }
            if reserved.len() == 3 {
                put(&mut m, "reserved", json!(reserved));
            }
        }
        Protocol::Socks { username, password } | Protocol::Http { username, password } => {
            if let Some(u) = username {
                put(&mut m, "username", u.clone().into());
            }
            if let Some(p) = password {
                put(&mut m, "password", p.clone().into());
            }
        }
        Protocol::Other { .. } => {}
    }

    if let Some(t) = &n.tls {
        let intrinsic = matches!(n.protocol, Protocol::Trojan { .. } | Protocol::Hysteria2 { .. } | Protocol::Tuic { .. } | Protocol::Anytls { .. });
        if !intrinsic {
            put(&mut m, "tls", true.into());
        }
        let sni_key = if matches!(n.protocol, Protocol::Vless { .. } | Protocol::Vmess { .. }) { "servername" } else { "sni" };
        if let Some(sni) = &t.sni {
            put(&mut m, sni_key, sni.clone().into());
        }
        if !t.alpn.is_empty() {
            put(&mut m, "alpn", json!(t.alpn));
        }
        if t.insecure {
            put(&mut m, "skip-cert-verify", true.into());
        }
        // Mihomo pins one certificate; Xray takes several.
        if let Some(pin) = t.pin.as_deref().and_then(|p| p.split(',').next()) {
            put(&mut m, "fingerprint", pin.into());
        }
        // Reality needs one; Russian DPI holds Chrome's more often than Firefox's.
        let fp = t.fingerprint.clone().or_else(|| t.reality.as_ref().map(|_| "firefox".to_string()));
        if let Some(fp) = fp {
            if !matches!(n.protocol, Protocol::Hysteria2 { .. } | Protocol::Tuic { .. }) {
                put(&mut m, "client-fingerprint", fp.into());
            }
        }
        if let Some(r) = &t.reality {
            let mut ro = Map::new();
            ro.insert("public-key".into(), r.public_key.clone().into());
            if let Some(sid) = &r.short_id {
                ro.insert("short-id".into(), sid.clone().into());
            }
            put(&mut m, "reality-opts", Value::Object(ro));
        }
    }

    match &n.transport {
        Transport::Tcp => {}
        Transport::Ws { path, host } | Transport::HttpUpgrade { path, host } => {
            put(&mut m, "network", "ws".into());
            let mut o = json!({ "path": path });
            if let Some(h) = host {
                o["headers"] = json!({ "Host": h });
            }
            if matches!(n.transport, Transport::HttpUpgrade { .. }) {
                o["v2ray-http-upgrade"] = true.into();
                o["v2ray-http-upgrade-fast-open"] = true.into();
            }
            put(&mut m, "ws-opts", o);
        }
        Transport::Grpc { service_name } => {
            put(&mut m, "network", "grpc".into());
            put(&mut m, "grpc-opts", json!({ "grpc-service-name": service_name }));
        }
        Transport::H2 { path, host } => {
            put(&mut m, "network", "h2".into());
            put(&mut m, "h2-opts", json!({ "path": path, "host": host.iter().collect::<Vec<_>>() }));
        }
        Transport::Xhttp { path, host, mode, .. } => {
            put(&mut m, "network", "xhttp".into());
            let mut o = json!({ "path": path });
            if let Some(h) = host {
                o["host"] = h.clone().into();
            }
            if let Some(md) = mode {
                o["mode"] = md.clone().into();
            }
            put(&mut m, "xhttp-opts", o);
        }
    }
    Value::Object(m)
}

/// A `proxy-groups:` entry, with its type named the way Mihomo's API reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClashGroup {
    pub name: String,
    /// `Selector`, `URLTest`, `Fallback`, `LoadBalance` or `Relay`.
    pub kind: String,
    /// Members listed inline. Members pulled in from providers are not known offline.
    pub members: Vec<String>,
}

/// The proxy groups of a Clash config, in file order. Empty if it can't be read.
pub fn groups(raw: &str) -> Vec<ClashGroup> {
    let Ok(doc) = serde_yaml_ng::from_str::<Value>(raw) else { return Vec::new() };
    doc.get("proxy-groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let kind = match s(g, "type")?.as_str() {
                "select" => "Selector",
                "url-test" => "URLTest",
                "fallback" => "Fallback",
                "load-balance" => "LoadBalance",
                "relay" => "Relay",
                other => other,
            }
            .to_string();
            Some(ClashGroup { name: s(g, "name")?, kind, members: list(g, "proxies") })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::parse_link;

    #[test]
    fn reads_groups_in_order() {
        let g = groups("proxy-groups:\n  - {name: Main, type: select, proxies: [Auto, A]}\n  - {name: Auto, type: url-test, proxies: [A]}\n");
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], ClashGroup { name: "Main".into(), kind: "Selector".into(), members: vec!["Auto".into(), "A".into()] });
        assert_eq!(g[1].kind, "URLTest");
        assert!(groups("not: [valid").is_empty());
    }

    #[test]
    fn clash_roundtrip_preserves_nodes() {
        for l in [
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.42:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=KEY&sid=6ba85179&type=tcp#NL",
            "trojan://pw@192.0.2.33:443?security=tls&sni=t.example&type=grpc&serviceName=svc#GB",
            "hysteria2://secret@203.0.113.87:443/?sni=example.com&obfs=salamander&obfs-password=pw&mport=20000-30000#DE",
            "tuic://uuid:pass@198.51.100.120:443?congestion_control=bbr&sni=a.example#SE",
            "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@198.51.100.1:8388#SS",
            "vless://id@a.example:443?security=tls&type=ws&path=%2Fws&host=cdn.example#WS",
            "vless://id@a.example:443?security=tls&type=httpupgrade&path=%2Fup#HU",
        ] {
            let n = parse_link(l).unwrap();
            let back = from_clash(&to_clash(&n)).unwrap();
            assert_eq!(n, back, "clash roundtrip failed for {l}");
        }
    }

    #[test]
    fn unknown_types_are_kept_verbatim() {
        let p = json!({"name": "old", "type": "ssr", "server": "a", "port": 1, "obfs": "plain"});
        let n = from_clash(&p).unwrap();
        assert!(matches!(&n.protocol, Protocol::Other { kind, .. } if kind == "ssr"));
        assert_eq!(to_clash(&n)["obfs"], "plain");
    }
}
