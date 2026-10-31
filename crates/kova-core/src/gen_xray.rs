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
                            // Russian DPI holds Chrome's fingerprint more often than Firefox's.
                            "fingerprint": t.fingerprint.clone().unwrap_or_else(|| "firefox".into()),
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
                    if let Some(pin) = &t.pin {
                        tls["pinnedPeerCertSha256"] = pin.clone().into();
                    }
                    // Certificate checks off: Xray no longer has "allowInsecure"; the
                    // certificate is pinned instead (see `with_pin`).
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

/// `outbound(n)`, with the certificate pinned when the link turns certificate checks
/// off (Xray 26.10 refuses "allowInsecure"; a pin trusts exactly that server's
/// certificate instead of any).
fn with_pin(n: &Node, opts: &RuntimeOptions) -> Option<Value> {
    let mut o = outbound(n)?;
    if n.tls.as_ref().is_some_and(|t| t.insecure && t.reality.is_none() && t.pin.is_none()) {
        if let Some(pin) = opts.cert_pins.get(&n.name) {
            o["streamSettings"]["tlsSettings"]["pinnedPeerCertSha256"] = pin.clone().into();
        }
    }
    Some(o)
}

/// The Anti-DPI outbound: a direct connection whose TLS handshake goes out in small
/// pieces with short pauses, and whose UDP starts with a few random packets, so DPI that
/// reads one packet at a time can't match it.
pub const ANTI_DPI_TAG: &str = "anti-dpi";

/// The Direct outbound every server connection is dialled through with Anti-DPI: the TLS
/// greeting goes out in pieces of 100–200 bytes 10–20 ms apart, and UDP starts with
/// 10–20 random bytes. Done by Xray's Finalmask; Xray older than v26.3.23 has no
/// Finalmask fragment or noise and gets the same through the outbound's own settings.
fn anti_dpi_outbound(opts: &RuntimeOptions) -> Value {
    if opts.xray_legacy_masks {
        return json!({
            "tag": ANTI_DPI_TAG,
            "protocol": "freedom",
            "settings": {
                "fragment": { "packets": "tlshello", "length": "100-200", "interval": "10-20" },
                "noises": [{ "type": "rand", "packet": "10-20", "delay": "10-16" }],
            },
        });
    }
    json!({
        "tag": ANTI_DPI_TAG,
        "protocol": "freedom",
        "settings": {},
        "streamSettings": {
            "finalmask": {
                "tcp": [{ "type": "fragment", "settings": { "packets": "tlshello", "length": "100-200", "delay": "10-20" } }],
                "udp": [{ "type": "noise", "settings": { "noise": [{ "rand": "10-20", "delay": "10-16" }] } }],
            },
        },
    })
}

/// The first Xray with Finalmask's fragment and noise.
pub const FINALMASK_SINCE: (u32, u32, u32) = (26, 3, 23);

/// Whether an Xray of `version` ("26.10.21" or "v26.10.21") predates them. An unknown
/// version counts as new: Kova's builds ship a newer one.
pub fn legacy_masks(version: Option<&str>) -> bool {
    let Some(v) = version else { return false };
    let mut parts = v.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten(), parts.next().flatten()) {
        (Some(a), Some(b), Some(c)) => (a, b, c) < FINALMASK_SINCE,
        _ => false,
    }
}

/// Sidecar for a Mihomo-fronted profile: one local SOCKS inbound per bridged node, and
/// with Anti-DPI one more that Mihomo dials every server through.
pub fn sidecar(bridged: &[(&Node, u16)], opts: &RuntimeOptions) -> String {
    let mut inbounds = Vec::new();
    let mut outbounds = Vec::new();
    let mut rules = Vec::new();
    // Only Mihomo uses these ports; with a login no other local program can.
    let settings = if opts.local_password.is_empty() {
        json!({ "udp": true, "auth": "noauth" })
    } else {
        json!({ "udp": true, "auth": "password", "accounts": [{ "user": crate::options::LOCAL_USER, "pass": opts.local_password }] })
    };
    for (i, (n, port)) in bridged.iter().enumerate() {
        let Some(mut o) = with_pin(n, opts) else { continue };
        if opts.anti_dpi_port.is_some() {
            o["streamSettings"]["sockopt"]["dialerProxy"] = ANTI_DPI_TAG.into();
        }
        let tag = format!("in-{i}");
        inbounds.push(json!({ "tag": tag, "listen": "127.0.0.1", "port": port, "protocol": "socks", "settings": settings }));
        rules.push(json!({ "inboundTag": [tag], "outboundTag": n.name }));
        outbounds.push(o);
    }
    if let Some(port) = opts.anti_dpi_port {
        inbounds.push(json!({ "tag": "in-anti-dpi", "listen": "127.0.0.1", "port": port, "protocol": "socks", "settings": settings }));
        rules.push(json!({ "inboundTag": ["in-anti-dpi"], "outboundTag": ANTI_DPI_TAG }));
        outbounds.push(anti_dpi_outbound(opts));
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

/// What every server's outbound tag starts with when Xray is the front core. Xray picks
/// the servers for Auto (`observatory`, `balancers`) by tag prefix: with the servers'
/// own names a server called "d" would also take in "direct" and Auto could choose it.
pub const NODE_TAG: &str = "node:";

pub fn node_tag(name: &str) -> String {
    format!("{NODE_TAG}{name}")
}

/// The server name of an outbound tag from [`full`].
pub fn node_of_tag(tag: &str) -> &str {
    tag.strip_prefix(NODE_TAG).unwrap_or(tag)
}

/// Which outbound the main `Proxy` target resolves to when Xray is the front core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Fastest node by Xray's observatory.
    Auto,
    Node(String),
}

/// The (HTTP, SOCKS) ports other devices use when Xray is the front core and LAN sharing
/// is on. Xray can't skip the login for this computer the way Mihomo does, so the local
/// ports stay on loopback without one and the network gets these two, with Kova's login.
/// `None` without a login: then nothing is shared, rather than shared openly.
pub fn lan_ports(opts: &RuntimeOptions) -> Option<(u16, u16)> {
    if !opts.allow_lan || opts.local_password.is_empty() {
        return None;
    }
    Some((opts.mixed_port.checked_add(2)?, opts.mixed_port.checked_add(3)?))
}

/// Full Xray config when Xray is the front core.
pub fn full(nodes: &[Node], selection: &Selection, opts: &RuntimeOptions) -> String {
    let mut outbounds_nodes: Vec<Value> = nodes
        .iter()
        .filter_map(|n| {
            let mut o = with_pin(n, opts)?;
            o["tag"] = node_tag(&n.name).into();
            Some(o)
        })
        .collect();
    if opts.anti_dpi {
        for o in &mut outbounds_nodes {
            o["streamSettings"]["sockopt"]["dialerProxy"] = ANTI_DPI_TAG.into();
        }
    }
    let tags: Vec<String> = outbounds_nodes.iter().filter_map(|o| o["tag"].as_str().map(String::from)).collect();

    let proxy_target = |rule: &mut Value| match selection {
        Selection::Auto if !tags.is_empty() => {
            rule["balancerTag"] = AUTO.into();
        }
        Selection::Node(n) if tags.contains(&node_tag(n)) => {
            rule["outboundTag"] = node_tag(n).into();
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
            t if tags.contains(&node_tag(t)) => x["outboundTag"] = node_tag(t).into(),
            _ => proxy_target(&mut x),
        }
        rules.push(x);
    }

    let mut outbounds = outbounds_nodes;
    outbounds.push(json!({ "tag": "direct", "protocol": "freedom", "settings": { "domainStrategy": "UseIP" } }));
    outbounds.push(json!({ "tag": "block", "protocol": "blackhole" }));
    if opts.anti_dpi {
        outbounds.push(anti_dpi_outbound(opts));
    }

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
            { "tag": "http-in", "listen": "127.0.0.1", "port": opts.mixed_port, "protocol": "http", "sniffing": sniffing(opts) },
            { "tag": "socks-in", "listen": "127.0.0.1", "port": opts.mixed_port.saturating_add(1), "protocol": "socks", "settings": { "udp": true, "auth": "noauth" }, "sniffing": sniffing(opts) },
        ],
        "outbounds": outbounds,
        "routing": { "domainStrategy": "IPIfNonMatch", "rules": rules },
        "stats": {},
        "policy": { "system": { "statsInboundUplink": true, "statsInboundDownlink": true, "statsOutboundUplink": true, "statsOutboundDownlink": true } },
        "metrics": { "tag": "metrics", "listen": format!("127.0.0.1:{}", opts.xray_metrics_port) },
    });
    if let Some((http, socks)) = lan_ports(opts) {
        let account = json!({ "user": crate::options::LOCAL_USER, "pass": opts.local_password });
        let inbounds = doc["inbounds"].as_array_mut().expect("inbounds");
        inbounds.push(json!({ "tag": "http-lan", "listen": "0.0.0.0", "port": http, "protocol": "http", "settings": { "accounts": [account] }, "sniffing": sniffing(opts) }));
        inbounds.push(json!({ "tag": "socks-lan", "listen": "0.0.0.0", "port": socks, "protocol": "socks", "settings": { "udp": true, "auth": "password", "accounts": [account] }, "sniffing": sniffing(opts) }));
    }
    if !tags.is_empty() {
        doc["observatory"] = json!({ "subjectSelector": [NODE_TAG], "probeUrl": TEST_URL, "probeInterval": "5m", "enableConcurrency": true });
        doc["routing"]["balancers"] = json!([{ "tag": AUTO, "selector": [NODE_TAG], "strategy": { "type": "leastPing" }, "fallbackTag": tags[0] }]);
    }
    serde_json::to_string_pretty(&doc).expect("valid json")
}

#[cfg(test)]
mod tests {
    #[test]
    fn insecure_links_are_pinned_not_allow_insecure() {
        let n = crate::links::parse_link("vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls&sni=a.example&allowInsecure=1&type=tcp#Self-signed").unwrap();
        let mut o = RuntimeOptions::default();
        o.cert_pins.insert("Self-signed".into(), "ab12".into());
        let doc: Value = serde_json::from_str(&full(std::slice::from_ref(&n), &Selection::Auto, &o)).unwrap();
        let tls = &doc["outbounds"][0]["streamSettings"]["tlsSettings"];
        assert_eq!(tls["pinnedPeerCertSha256"], "ab12");
        assert!(tls.get("allowInsecure").is_none(), "Xray 26.10 refuses allowInsecure: {tls}");
        // No pin (the server didn't answer): the certificate is checked normally.
        let doc: Value = serde_json::from_str(&full(&[n], &Selection::Auto, &RuntimeOptions::default())).unwrap();
        assert!(doc.to_string().find("allowInsecure").is_none());
    }

    #[test]
    fn anti_dpi_with_xray_in_front() {
        let o = RuntimeOptions { anti_dpi: true, ..Default::default() };
        let n = crate::links::parse_link("vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls&type=tcp#A").unwrap();
        let w = crate::links::parse_link("wireguard://pk@203.0.113.2:51820?publickey=PUB&address=10.0.0.2%2F32#W");
        let mut nodes = vec![n];
        nodes.extend(w);
        let doc: Value = serde_json::from_str(&full(&nodes, &Selection::Auto, &o)).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        for o in outbounds.iter().filter(|o| o["protocol"] != "freedom" && o["protocol"] != "blackhole") {
            assert_eq!(o["streamSettings"]["sockopt"]["dialerProxy"], ANTI_DPI_TAG, "{o}");
        }
        assert!(outbounds.iter().any(|o| o["tag"] == ANTI_DPI_TAG && o["streamSettings"]["finalmask"]["tcp"][0]["settings"]["packets"] == "tlshello"));
        // Not among the servers the balancer picks from.
        assert!(!doc["routing"]["balancers"][0]["selector"].as_array().unwrap().iter().any(|t| t == ANTI_DPI_TAG));
    }

    #[test]
    fn anti_dpi_uses_finalmask_and_the_old_way_only_for_old_xray() {
        let n = parse_link("vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls&type=tcp#A").unwrap();
        let anti = |o: &RuntimeOptions| -> Value {
            let doc: Value = serde_json::from_str(&sidecar(&[(&n, 17001)], o)).unwrap();
            doc["outbounds"].as_array().unwrap().iter().find(|x| x["tag"] == ANTI_DPI_TAG).cloned().unwrap()
        };
        let mut o = RuntimeOptions { anti_dpi: true, anti_dpi_port: Some(17100), ..Default::default() };
        let now = anti(&o);
        assert_eq!(now["settings"], json!({}), "no deprecated Direct settings: {now}");
        assert_eq!(
            now["streamSettings"]["finalmask"],
            json!({
                "tcp": [{ "type": "fragment", "settings": { "packets": "tlshello", "length": "100-200", "delay": "10-20" } }],
                "udp": [{ "type": "noise", "settings": { "noise": [{ "rand": "10-20", "delay": "10-16" }] } }],
            })
        );

        o.xray_legacy_masks = true;
        let old = anti(&o);
        assert!(old.get("streamSettings").is_none());
        assert_eq!(old["settings"]["fragment"], json!({ "packets": "tlshello", "length": "100-200", "interval": "10-20" }));
        assert_eq!(old["settings"]["noises"][0]["packet"], "10-20");

        for (v, legacy) in [("26.10.21", false), ("v26.10.29", false), ("26.3.23", false), ("26.3.22", true), ("26.2.6", true), ("25.12.8", true)] {
            assert_eq!(legacy_masks(Some(v)), legacy, "{v}");
        }
        assert!(!legacy_masks(None), "unknown: Kova's builds ship a newer Xray");
        assert!(!legacy_masks(Some("dev")));
    }

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
    fn lan_sharing_needs_the_login() {
        let nodes = vec![parse_link("trojan://pw@b.example:443#TR").unwrap()];
        let inbounds = |o: &RuntimeOptions| -> Vec<Value> {
            let doc: Value = serde_json::from_str(&full(&nodes, &Selection::Auto, o)).unwrap();
            doc["inbounds"].as_array().unwrap().clone()
        };
        let mut o = RuntimeOptions { allow_lan: true, ..Default::default() };
        let without_login = inbounds(&o);
        assert_eq!(without_login.len(), 2, "no login, nothing shared");
        assert!(without_login.iter().all(|i| i["listen"] == "127.0.0.1"));

        o.local_password = "pw".into();
        let shared = inbounds(&o);
        assert!(shared[..2].iter().all(|i| i["listen"] == "127.0.0.1"), "this computer's ports stay local, without a login");
        assert_eq!((&shared[2]["tag"], &shared[2]["port"], &shared[2]["listen"]), (&json!("http-lan"), &json!(7892), &json!("0.0.0.0")));
        assert_eq!(shared[2]["settings"]["accounts"], json!([{ "user": "kova", "pass": "pw" }]));
        assert_eq!((&shared[3]["port"], &shared[3]["settings"]["auth"]), (&json!(7893), &json!("password")));

        o.allow_lan = false;
        assert_eq!(inbounds(&o).len(), 2);
        o.allow_lan = true;
        o.mixed_port = 65534;
        assert_eq!(lan_ports(&o), None, "no room above the proxy port");
    }

    #[test]
    fn sidecar_maps_inbounds_to_nodes() {
        let n = parse_link("vless://u@a:443?security=tls&type=xhttp&path=/p&mode=packet-up#X").unwrap();
        let doc: Value = serde_json::from_str(&sidecar(&[(&n, 17001)], &RuntimeOptions::default())).unwrap();
        assert_eq!(doc["inbounds"][0]["port"], 17001);
        assert_eq!(doc["routing"]["rules"][0]["outboundTag"], "X");
        assert_eq!(doc["outbounds"][0]["streamSettings"]["xhttpSettings"]["mode"], "packet-up");
    }

    /// Auto picks servers by tag prefix: a server named "d" must not take in "direct".
    #[test]
    fn auto_only_picks_servers() {
        let nodes = vec![crate::links::parse_link("trojan://pw@203.0.113.5:443?security=tls#d").unwrap()];
        let doc: Value = serde_json::from_str(&full(&nodes, &Selection::Node("d".into()), &RuntimeOptions::default())).unwrap();
        let tags: Vec<&str> = doc["outbounds"].as_array().unwrap().iter().filter_map(|o| o["tag"].as_str()).collect();
        assert!(tags.iter().filter(|t| t.starts_with(NODE_TAG)).eq(["node:d"].iter()), "{tags:?}");
        assert_eq!(doc["observatory"]["subjectSelector"], json!([NODE_TAG]));
        assert_eq!(doc["routing"]["balancers"][0]["selector"], json!([NODE_TAG]));
        assert!(doc["routing"]["rules"].as_array().unwrap().iter().any(|r| r["outboundTag"] == "node:d"), "the chosen server");
        assert_eq!(node_of_tag("node:d"), "d");
    }
}
