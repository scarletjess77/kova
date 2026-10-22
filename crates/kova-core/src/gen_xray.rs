//! Xray config generation.

use serde_json::{Value, json};

use crate::model::*;
use crate::options::*;

fn stream(n: &Node) -> Value {
    let mut s = serde_json::Map::new();
    let network = match &n.transport {
        Transport::Tcp => "raw",
        Transport::Ws { .. } => "ws",
        Transport::Grpc { .. } => "grpc",
        Transport::H2 { .. } => "xhttp",
        Transport::HttpUpgrade { .. } => "httpupgrade",
        Transport::Xhttp { .. } => "xhttp",
    };
    s.insert("network".into(), network.into());
    match &n.transport {
        Transport::Tcp => {}
        Transport::Ws { path, host } => {
            s.insert("wsSettings".into(), json!({ "path": path, "host": host.clone().unwrap_or_default() }));
        }
        Transport::Grpc { service_name } => {
            s.insert("grpcSettings".into(), json!({ "serviceName": service_name }));
        }
        Transport::HttpUpgrade { path, host } => {
            s.insert("httpupgradeSettings".into(), json!({ "path": path, "host": host.clone().unwrap_or_default() }));
        }
        Transport::Xhttp { path, host, mode, extra } => {
            let mut x = json!({ "path": path, "mode": mode.clone().unwrap_or_else(|| "auto".into()) });
            if let Some(h) = host {
                x["host"] = h.clone().into();
            }
            if let Some(e) = extra {
                x["extra"] = e.clone();
            }
            s.insert("xhttpSettings".into(), x);
        }
        Transport::H2 { path, host } => {
            s.insert("xhttpSettings".into(), json!({ "path": path, "host": host.clone().unwrap_or_default(), "mode": "stream-one" }));
        }
    }
    match &n.tls {
        None => {
            s.insert("security".into(), "none".into());
        }
        Some(t) => {
            let sni = t.sni.clone().unwrap_or_else(|| n.server.clone());
            match &t.reality {
                Some(r) => {
                    s.insert("security".into(), "reality".into());
                    s.insert(
                        "realitySettings".into(),
                        json!({
                            "serverName": sni,
                            "fingerprint": t.fingerprint.clone().unwrap_or_else(|| "chrome".into()),
                            "publicKey": r.public_key,
                            "shortId": r.short_id.clone().unwrap_or_default(),
                            "spiderX": r.spider_x.clone().unwrap_or_default(),
                        }),
                    );
                }
                None => {
                    s.insert("security".into(), "tls".into());
                    let mut tls = json!({ "serverName": sni });
                    if !t.alpn.is_empty() {
                        tls["alpn"] = json!(t.alpn);
                    }
                    if let Some(fp) = &t.fingerprint {
                        tls["fingerprint"] = fp.clone().into();
                    }
                    if t.insecure {
                        tls["allowInsecure"] = true.into();
                    }
                    s.insert("tlsSettings".into(), tls);
                }
            }
        }
    }
    Value::Object(s)
}

/// One Xray outbound for `n`, tagged with the node name. `None` if Xray can't run it.
pub fn outbound(n: &Node) -> Option<Value> {
    let settings = match &n.protocol {
        Protocol::Vless { uuid, flow, encryption } => json!({ "vnext": [{ "address": n.server, "port": n.port, "users": [{
            "id": uuid, "encryption": encryption.clone().unwrap_or_else(|| "none".into()), "flow": flow.clone().unwrap_or_default()
        }]}]}),
        Protocol::Vmess { uuid, cipher, .. } => json!({ "vnext": [{ "address": n.server, "port": n.port, "users": [{ "id": uuid, "security": cipher }]}]}),
        Protocol::Trojan { password } => json!({ "servers": [{ "address": n.server, "port": n.port, "password": password }]}),
        Protocol::Shadowsocks { method, password, plugin: None } => {
            json!({ "servers": [{ "address": n.server, "port": n.port, "method": method, "password": password, "uot": true }]})
        }
        Protocol::Wireguard { private_key, public_key, pre_shared_key, addresses, mtu, reserved } => {
            let mut peer = json!({ "publicKey": public_key, "endpoint": format!("{}:{}", host(&n.server), n.port), "allowedIPs": ["0.0.0.0/0", "::/0"] });
            if let Some(p) = pre_shared_key {
                peer["preSharedKey"] = p.clone().into();
            }
            let mut w = json!({ "secretKey": private_key, "address": addresses, "peers": [peer], "mtu": mtu.unwrap_or(1420), "noKernelTun": true });
            if reserved.len() == 3 {
                w["reserved"] = json!(reserved);
            }
            w
        }
        Protocol::Socks { username, password } => {
            let mut srv = json!({ "address": n.server, "port": n.port });
            if let (Some(u), Some(p)) = (username, password) {
                srv["users"] = json!([{ "user": u, "pass": p }]);
            }
            json!({ "servers": [srv] })
        }
        Protocol::Http { username, password } => {
            let mut srv = json!({ "address": n.server, "port": n.port });
            if let (Some(u), Some(p)) = (username, password) {
                srv["users"] = json!([{ "user": u, "pass": p }]);
            }
            json!({ "servers": [srv] })
        }
        _ => return None,
    };
    let protocol = match &n.protocol {
        Protocol::Vless { .. } => "vless",
        Protocol::Vmess { .. } => "vmess",
        Protocol::Trojan { .. } => "trojan",
        Protocol::Shadowsocks { .. } => "shadowsocks",
        Protocol::Wireguard { .. } => "wireguard",
        Protocol::Socks { .. } => "socks",
        Protocol::Http { .. } => "http",
        _ => unreachable!(),
    };
    let mut o = json!({ "tag": n.name, "protocol": protocol, "settings": settings });
    if !matches!(n.protocol, Protocol::Wireguard { .. }) {
        o["streamSettings"] = stream(n);
    }
    Some(o)
}

fn host(h: &str) -> String {
    if h.contains(':') { format!("[{h}]") } else { h.to_string() }
}

fn log(opts: &RuntimeOptions) -> Value {
    let level = match opts.log_level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warning => "warning",
        LogLevel::Error => "error",
        LogLevel::Silent => "none",
    };
    json!({ "loglevel": level, "access": "" })
}

fn sniffing(opts: &RuntimeOptions) -> Value {
    json!({ "enabled": opts.sniff, "destOverride": ["http", "tls", "quic"], "routeOnly": true })
}

/// Sidecar for a Mihomo-fronted profile: one local SOCKS inbound per bridged node.
pub fn sidecar(bridged: &[(&Node, u16)], opts: &RuntimeOptions) -> String {
    let mut inbounds = Vec::new();
    let mut outbounds = Vec::new();
    let mut rules = Vec::new();
    for (i, (n, port)) in bridged.iter().enumerate() {
        let Some(o) = outbound(n) else { continue };
        let tag = format!("in-{i}");
        // Only Mihomo uses the bridge; with a login no other local program can.
        let settings = if opts.local_password.is_empty() {
            json!({ "udp": true, "auth": "noauth" })
        } else {
            json!({ "udp": true, "auth": "password", "accounts": [{ "user": crate::options::LOCAL_USER, "pass": opts.local_password }] })
        };
        inbounds.push(json!({ "tag": tag, "listen": "127.0.0.1", "port": port, "protocol": "socks", "settings": settings }));
        rules.push(json!({ "inboundTag": [tag], "outboundTag": n.name }));
        outbounds.push(o);
    }
    outbounds.push(json!({ "tag": "block", "protocol": "blackhole" }));
    let doc = json!({
        "log": log(opts),
        "inbounds": inbounds,
        "outbounds": outbounds,
        "routing": { "rules": rules },
    });
    serde_json::to_string_pretty(&doc).expect("valid json")
}

/// Which outbound the main `Proxy` target resolves to when Xray is the front core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Fastest node by Xray's observatory.
    Auto,
    Node(String),
}

/// Full Xray config when Xray is the front core.
pub fn full(nodes: &[Node], selection: &Selection, opts: &RuntimeOptions) -> String {
    let listen = opts.listen_addr();
    let outbounds_nodes: Vec<Value> = nodes.iter().filter_map(outbound).collect();
    let tags: Vec<String> = outbounds_nodes.iter().filter_map(|o| o["tag"].as_str().map(String::from)).collect();

    let proxy_target = |rule: &mut Value| match selection {
        Selection::Auto if !tags.is_empty() => {
            rule["balancerTag"] = AUTO.into();
        }
        Selection::Node(n) if tags.contains(n) => {
            rule["outboundTag"] = n.clone().into();
        }
        _ => {
            rule["outboundTag"] = tags.first().cloned().unwrap_or_else(|| "direct".into()).into();
        }
    };

    let mut rules: Vec<Value> = Vec::new();
    let all: Vec<Rule> = match opts.mode {
        Mode::Rule => opts.rules().into_iter().chain(opts.tail_rules()).collect(),
        Mode::Global => vec![Rule { kind: RuleKind::Match, value: String::new(), target: PROXY.into() }],
        Mode::Direct => vec![Rule { kind: RuleKind::Match, value: String::new(), target: "DIRECT".into() }],
    };
    for r in all {
        let mut x = match r.kind {
            RuleKind::Domain => json!({ "domain": [format!("full:{}", r.value)] }),
            RuleKind::DomainSuffix => json!({ "domain": [format!("domain:{}", r.value)] }),
            RuleKind::DomainKeyword => json!({ "domain": [format!("keyword:{}", r.value)] }),
            RuleKind::Geosite => json!({ "domain": [format!("geosite:{}", r.value)] }),
            RuleKind::Geoip => json!({ "ip": [format!("geoip:{}", r.value.to_lowercase())] }),
            RuleKind::IpCidr => json!({ "ip": [r.value] }),
            RuleKind::ProcessName => json!({ "process": [r.value] }),
            RuleKind::Match => json!({ "network": "tcp,udp" }),
        };
        match r.target.as_str() {
            "DIRECT" => x["outboundTag"] = "direct".into(),
            "REJECT" => x["outboundTag"] = "block".into(),
            t if t == PROXY || t == AUTO || t == FALLBACK => proxy_target(&mut x),
            t if tags.iter().any(|g| g == t) => x["outboundTag"] = t.into(),
            _ => proxy_target(&mut x),
        }
        rules.push(x);
    }

    let mut outbounds = outbounds_nodes;
    outbounds.push(json!({ "tag": "direct", "protocol": "freedom", "settings": { "domainStrategy": "UseIP" } }));
    outbounds.push(json!({ "tag": "block", "protocol": "blackhole" }));

    let mut dns_servers: Vec<Value> = opts.dns.nameservers.iter().map(|s| Value::String(s.clone())).collect();
    let direct_domains: Vec<String> = opts.presets.iter().flat_map(|p| p.direct_geosites()).map(|g| format!("geosite:{g}")).collect();
    if !direct_domains.is_empty() {
        if let Some(d) = opts.dns.direct_nameservers.first() {
            dns_servers.push(json!({ "address": d, "domains": direct_domains, "skipFallback": true }));
        }
    }

    let mut doc = json!({
        "log": log(opts),
        "dns": { "servers": dns_servers, "queryStrategy": if opts.ipv6 { "UseIP" } else { "UseIPv4" } },
        "inbounds": [
            { "tag": "http-in", "listen": listen, "port": opts.mixed_port, "protocol": "http", "sniffing": sniffing(opts) },
            { "tag": "socks-in", "listen": listen, "port": opts.mixed_port.saturating_add(1), "protocol": "socks", "settings": { "udp": true, "auth": "noauth" }, "sniffing": sniffing(opts) },
        ],
        "outbounds": outbounds,
        "routing": { "domainStrategy": "IPIfNonMatch", "rules": rules },
        "stats": {},
        "policy": { "system": { "statsInboundUplink": true, "statsInboundDownlink": true, "statsOutboundUplink": true, "statsOutboundDownlink": true } },
        "metrics": { "tag": "metrics", "listen": format!("127.0.0.1:{}", opts.xray_metrics_port) },
    });
    if !tags.is_empty() {
        doc["observatory"] = json!({ "subjectSelector": tags, "probeUrl": TEST_URL, "probeInterval": "5m", "enableConcurrency": true });
        doc["routing"]["balancers"] = json!([{ "tag": AUTO, "selector": tags, "strategy": { "type": "leastPing" }, "fallbackTag": tags[0] }]);
    }
    serde_json::to_string_pretty(&doc).expect("valid json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::parse_link;

    #[test]
    fn reality_vision_outbound() {
        let n = parse_link("vless://u@203.0.113.42:443?flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=KEY&sid=ab#NL").unwrap();
        let o = outbound(&n).unwrap();
        assert_eq!(o["protocol"], "vless");
        assert_eq!(o["settings"]["vnext"][0]["users"][0]["flow"], "xtls-rprx-vision");
        assert_eq!(o["streamSettings"]["security"], "reality");
        assert_eq!(o["streamSettings"]["realitySettings"]["publicKey"], "KEY");
        assert_eq!(o["streamSettings"]["network"], "raw");
    }

    #[test]
    fn full_config_routes_presets_and_balancer() {
        let nodes = vec![
            parse_link("trojan://pw@a.example:443#A").unwrap(),
            parse_link("hy2://pw@b.example:443#B").unwrap(),
        ];
        let doc: Value = serde_json::from_str(&full(&nodes, &Selection::Auto, &RuntimeOptions::default())).unwrap();
        let outs = doc["outbounds"].as_array().unwrap();
        assert_eq!(outs.len(), 3, "hysteria2 is skipped, direct and block added");
        let rules = doc["routing"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["ip"][0] == "geoip:ru" && r["outboundTag"] == "direct"));
        assert_eq!(rules.last().unwrap()["balancerTag"], AUTO);
        assert_eq!(doc["inbounds"][0]["port"], 7890);
    }

    #[test]
    fn sidecar_maps_inbounds_to_nodes() {
        let n = parse_link("vless://u@a:443?security=tls&type=xhttp&path=/p&mode=packet-up#X").unwrap();
        let doc: Value = serde_json::from_str(&sidecar(&[(&n, 17001)], &RuntimeOptions::default())).unwrap();
        assert_eq!(doc["inbounds"][0]["port"], 17001);
        assert_eq!(doc["routing"]["rules"][0]["outboundTag"], "X");
        assert_eq!(doc["outbounds"][0]["streamSettings"]["xhttpSettings"]["mode"], "packet-up");
    }
}
