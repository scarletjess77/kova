//! Xray JSON -> [`Node`]: a full Xray config, or a list of them (the "xray-json"
//! subscription format of Remnawave and other panels, one config per server with its name
//! in `remarks`). Each config's proxy outbounds become nodes; utility outbounds (freedom,
//! blackhole, dns) are skipped.

use serde_json::Value;

use crate::model::*;

fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn port(v: &Value) -> Option<u16> {
    match v.get("port")? {
        Value::Number(n) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
        Value::String(p) => p.parse().ok(),
        _ => None,
    }
}

const SKIP: &[&str] = &["freedom", "blackhole", "dns", "loopback"];

/// Whether `doc` is Xray JSON rather than sing-box: Xray outbounds name their `protocol`,
/// sing-box ones their `type`.
pub fn looks_like(doc: &Value) -> bool {
    let config = match doc {
        Value::Array(a) => a.first(),
        d => Some(d),
    };
    config
        .and_then(|c| c.get("outbounds"))
        .and_then(Value::as_array)
        .is_some_and(|o| o.iter().any(|o| o.get("protocol").is_some()))
}

/// Returns the nodes and a warning per outbound that could not be converted.
pub fn parse(doc: &Value) -> (Vec<Node>, Vec<String>) {
    let configs: Vec<&Value> = match doc {
        Value::Array(a) => a.iter().collect(),
        d => vec![d],
    };
    let mut nodes = Vec::new();
    let mut warnings = Vec::new();
    for config in configs {
        let remarks = s(config, "remarks");
        // What Kova's own settings do instead, said once per config rather than dropped
        // silently: the provider's fragment (Anti-DPI) and mux.
        let label = remarks.clone().unwrap_or_else(|| "a server".into());
        let outbounds = config.get("outbounds").and_then(Value::as_array).cloned().unwrap_or_default();
        if outbounds.iter().any(|o| o.get("settings").and_then(|s| s.get("fragment")).is_some()) {
            warnings.push(format!("{label}: the provider's fragment setting isn't used; Settings → Anti-DPI does the same"));
        }
        if outbounds.iter().any(|o| o.get("mux").and_then(|m| m.get("enabled")).and_then(Value::as_bool) == Some(true)) {
            warnings.push(format!("{label}: the provider's mux setting isn't used"));
        }
        let proxies: Vec<&Value> = config
            .get("outbounds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|o| !SKIP.contains(&s(o, "protocol").unwrap_or_default().as_str()))
            .collect();
        for o in &proxies {
            let tag = s(o, "tag").unwrap_or_else(|| "proxy".into());
            if let Some(via) = o.get("streamSettings").and_then(|st| st.get("sockopt")).and_then(|so| s(so, "dialerProxy")) {
                warnings.push(format!("{label}: connects straight to the server, not through {via} as the provider chained it"));
            }
            // One server per config is the usual case: it takes the config's name.
            let name = match (&remarks, proxies.len()) {
                (Some(r), 1) => r.clone(),
                (Some(r), _) => format!("{r} · {tag}"),
                (None, _) => tag,
            };
            match convert(o, name) {
                Ok(n) => nodes.push(n),
                Err(e) => warnings.push(e),
            }
        }
    }
    (nodes, warnings)
}

/// The server entry of an outbound: `settings.vnext[0]` / `settings.servers[0]`, or the
/// flat form newer Xray accepts (`settings.address`, `settings.port`, …).
fn server_entry(settings: &Value) -> Option<&Value> {
    for k in ["vnext", "servers"] {
        if let Some(e) = settings.get(k).and_then(Value::as_array).and_then(|a| a.first()) {
            return Some(e);
        }
    }
    settings.get("address").map(|_| settings)
}

fn convert(o: &Value, name: String) -> Result<Node, String> {
    let kind = s(o, "protocol").unwrap_or_default();
    let label = name.clone();
    let fail = |why: &str| format!("{label}: {why}");
    let settings = o.get("settings").cloned().unwrap_or(Value::Null);

    if kind == "wireguard" {
        let peer = settings.get("peers").and_then(Value::as_array).and_then(|p| p.first()).ok_or_else(|| fail("WireGuard without a peer"))?;
        let endpoint = s(peer, "endpoint").ok_or_else(|| fail("WireGuard without an endpoint"))?;
        let (host, p) = endpoint.rsplit_once(':').ok_or_else(|| fail("bad WireGuard endpoint"))?;
        let addresses = settings.get("address").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        let reserved = settings.get("reserved").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u8)).collect()).unwrap_or_default();
        return Ok(Node {
            name,
            server: host.trim_matches(['[', ']']).to_string(),
            port: p.parse().map_err(|_| fail("bad WireGuard port"))?,
            protocol: Protocol::Wireguard {
                private_key: s(&settings, "secretKey").ok_or_else(|| fail("WireGuard without a key"))?,
                public_key: s(peer, "publicKey").ok_or_else(|| fail("WireGuard without the peer's key"))?,
                pre_shared_key: s(peer, "preSharedKey"),
                addresses,
                mtu: settings.get("mtu").and_then(Value::as_u64).map(|m| m as u16),
                reserved,
            },
            transport: Transport::Tcp,
            tls: None,
            udp: true,
        });
    }

    let entry = server_entry(&settings).ok_or_else(|| fail(&format!("{kind} outbound without a server")))?;
    let server = s(entry, "address").ok_or_else(|| fail("no server address"))?;
    let port = port(entry).ok_or_else(|| fail("no server port"))?;
    let user = entry.get("users").and_then(Value::as_array).and_then(|u| u.first()).unwrap_or(entry);
    let protocol = match kind.as_str() {
        "vless" => Protocol::Vless {
            uuid: s(user, "id").ok_or_else(|| fail("VLESS without an id"))?,
            flow: s(user, "flow"),
            encryption: s(user, "encryption").filter(|e| e != "none"),
        },
        "vmess" => Protocol::Vmess {
            uuid: s(user, "id").ok_or_else(|| fail("VMess without an id"))?,
            alter_id: user.get("alterId").and_then(Value::as_u64).unwrap_or(0) as u16,
            cipher: s(user, "security").unwrap_or_else(|| "auto".into()),
        },
        "trojan" => Protocol::Trojan { password: s(entry, "password").ok_or_else(|| fail("Trojan without a password"))? },
        "shadowsocks" => Protocol::Shadowsocks {
            method: s(entry, "method").ok_or_else(|| fail("Shadowsocks without a method"))?,
            password: s(entry, "password").ok_or_else(|| fail("Shadowsocks without a password"))?,
            plugin: None,
        },
        "socks" => Protocol::Socks { username: s(user, "user"), password: s(user, "pass") },
        "http" => Protocol::Http { username: s(user, "user"), password: s(user, "pass") },
        other => return Err(fail(&format!("{other} outbounds are not supported"))),
    };

    let stream = o.get("streamSettings").cloned().unwrap_or(Value::Null);
    let network = s(&stream, "network").unwrap_or_else(|| "tcp".into());
    let host_of = |v: &Value| s(v, "host").or_else(|| v.get("headers").and_then(|h| s(h, "Host")));
    let transport = match network.as_str() {
        "tcp" | "raw" => {
            let header = stream.get("tcpSettings").or_else(|| stream.get("rawSettings")).and_then(|t| t.get("header"));
            if header.and_then(|h| s(h, "type")).is_some_and(|t| t != "none") {
                return Err(fail("TCP with an HTTP header disguise is not supported"));
            }
            Transport::Tcp
        }
        "ws" => {
            let w = stream.get("wsSettings").cloned().unwrap_or(Value::Null);
            Transport::Ws { path: s(&w, "path").unwrap_or_else(|| "/".into()), host: host_of(&w) }
        }
        "grpc" => {
            let g = stream.get("grpcSettings").cloned().unwrap_or(Value::Null);
            Transport::Grpc { service_name: s(&g, "serviceName").unwrap_or_default() }
        }
        "httpupgrade" => {
            let h = stream.get("httpupgradeSettings").cloned().unwrap_or(Value::Null);
            Transport::HttpUpgrade { path: s(&h, "path").unwrap_or_else(|| "/".into()), host: host_of(&h) }
        }
        "xhttp" | "splithttp" => {
            let x = stream.get("xhttpSettings").or_else(|| stream.get("splithttpSettings")).cloned().unwrap_or(Value::Null);
            Transport::Xhttp { path: s(&x, "path").unwrap_or_else(|| "/".into()), host: s(&x, "host"), mode: s(&x, "mode"), extra: x.get("extra").cloned() }
        }
        "h2" | "http" => {
            let h = stream.get("httpSettings").cloned().unwrap_or(Value::Null);
            let host = h.get("host").and_then(Value::as_array).and_then(|a| a.first()).and_then(|v| v.as_str()).map(String::from);
            Transport::H2 { path: s(&h, "path").unwrap_or_else(|| "/".into()), host }
        }
        other => return Err(fail(&format!("the {other} transport is not supported"))),
    };

    let alpn = |t: &Value| t.get("alpn").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let tls = match s(&stream, "security").as_deref() {
        Some("tls") => {
            let t = stream.get("tlsSettings").cloned().unwrap_or(Value::Null);
            Some(Tls {
                sni: s(&t, "serverName"),
                alpn: alpn(&t),
                fingerprint: s(&t, "fingerprint"),
                insecure: t.get("allowInsecure").and_then(Value::as_bool).unwrap_or(false),
                reality: None,
            })
        }
        Some("reality") => {
            let r = stream.get("realitySettings").cloned().unwrap_or(Value::Null);
            // Newer Xray calls the public key `password`.
            let public_key = s(&r, "publicKey").or_else(|| s(&r, "password")).ok_or_else(|| fail("Reality without a public key"))?;
            Some(Tls {
                sni: s(&r, "serverName"),
                alpn: Vec::new(),
                fingerprint: s(&r, "fingerprint"),
                insecure: false,
                reality: Some(Reality { public_key, short_id: s(&r, "shortId"), spider_x: s(&r, "spiderX") }),
            })
        }
        _ => None,
    };

    Ok(Node { name, server, port, protocol, transport, tls, udp: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What a Remnawave panel sends for the xray-json format: one config per server.
    fn panel() -> Value {
        json!([
            {
                "remarks": "🇳🇱 Amsterdam",
                "outbounds": [
                    { "tag": "proxy", "protocol": "vless",
                      "settings": { "vnext": [{ "address": "203.0.113.1", "port": 443, "users": [{ "id": "b831381d-6324-4d53-ad4f-8cda48b30811", "flow": "xtls-rprx-vision", "encryption": "none" }] }] },
                      "streamSettings": { "network": "tcp", "security": "reality",
                                          "realitySettings": { "serverName": "www.example.com", "fingerprint": "firefox", "publicKey": "PUB", "shortId": "0123abcd" } } },
                    { "tag": "direct", "protocol": "freedom" },
                    { "tag": "block", "protocol": "blackhole" }
                ]
            },
            {
                "remarks": "🇳🇱 Amsterdam XHTTP",
                "outbounds": [
                    { "tag": "proxy", "protocol": "vless",
                      "settings": { "address": "203.0.113.1", "port": 8443, "id": "b831381d-6324-4d53-ad4f-8cda48b30811" },
                      "streamSettings": { "network": "xhttp", "security": "reality",
                                          "xhttpSettings": { "path": "/xh", "mode": "auto" },
                                          "realitySettings": { "serverName": "cdn.example.com", "password": "PUB2", "shortId": "4246" } } },
                    { "tag": "direct", "protocol": "freedom" }
                ]
            },
            {
                "remarks": "Old header",
                "outbounds": [{ "tag": "proxy", "protocol": "vmess",
                                "settings": { "vnext": [{ "address": "a.example", "port": 80, "users": [{ "id": "x" }] }] },
                                "streamSettings": { "network": "tcp", "tcpSettings": { "header": { "type": "http" } } } }]
            }
        ])
    }

    #[test]
    fn panel_configs_become_nodes() {
        assert!(looks_like(&panel()));
        let (nodes, warnings) = parse(&panel());
        assert_eq!(nodes.len(), 2, "{warnings:?}");
        assert_eq!(warnings.len(), 1, "the HTTP header disguise is reported, not dropped silently");

        let a = &nodes[0];
        assert_eq!((a.name.as_str(), a.server.as_str(), a.port), ("🇳🇱 Amsterdam", "203.0.113.1", 443));
        assert!(matches!(&a.protocol, Protocol::Vless { flow: Some(f), encryption: None, .. } if f == "xtls-rprx-vision"));
        let tls = a.tls.as_ref().unwrap();
        assert_eq!((tls.sni.as_deref(), tls.fingerprint.as_deref()), (Some("www.example.com"), Some("firefox")));
        assert_eq!(tls.reality.as_ref().unwrap().short_id.as_deref(), Some("0123abcd"));

        let x = &nodes[1];
        assert!(matches!(&x.transport, Transport::Xhttp { path, mode: Some(m), .. } if path == "/xh" && m == "auto"));
        assert_eq!(x.tls.as_ref().unwrap().reality.as_ref().unwrap().public_key, "PUB2", "the newer `password` name");
    }

    #[test]
    fn provider_settings_kova_does_not_use_are_reported() {
        let doc = json!([{
            "remarks": "Chained",
            "outbounds": [
                { "tag": "proxy", "protocol": "trojan", "mux": { "enabled": true },
                  "settings": { "servers": [{ "address": "203.0.113.5", "port": 443, "password": "x" }] },
                  "streamSettings": { "network": "tcp", "security": "tls", "sockopt": { "dialerProxy": "frag" } } },
                { "tag": "frag", "protocol": "freedom", "settings": { "fragment": { "packets": "tlshello" } } }
            ]
        }]);
        let (nodes, warnings) = parse(&doc);
        assert_eq!(nodes.len(), 1);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn sing_box_json_is_not_mistaken_for_xray() {
        assert!(!looks_like(&json!({ "outbounds": [{ "type": "vless", "tag": "a", "server": "x", "server_port": 443 }] })));
    }
}
