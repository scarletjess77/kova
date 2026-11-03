//! Turning a downloaded subscription body (or pasted text) into nodes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Node, dedupe_names};
use crate::{clash, links, singbox, xray_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// A full Clash / Mihomo config. Kept verbatim so its groups and rules survive.
    Clash,
    SingBox,
    /// An Xray config, or a list of them (one per server, as panels send it).
    Xray,
    /// Plain or base64 list of share links.
    Links,
    /// A WireGuard `.conf` file, such as a Cloudflare WARP profile.
    #[serde(rename = "wireguard")]
    WireGuard,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Clash => "Clash / Mihomo YAML",
            Format::SingBox => "sing-box JSON",
            Format::Xray => "Xray JSON",
            Format::Links => "Share links",
            Format::WireGuard => "WireGuard config",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub format: Format,
    pub nodes: Vec<Node>,
    /// Lines or entries that were skipped, with why.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the content has no proxies we can read")]
    Empty,
    #[error("invalid Clash config: {0}")]
    Clash(String),
}

/// Detect the format of `body` and extract its nodes.
pub fn parse(body: &str) -> Result<Parsed, ParseError> {
    let body = body.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        return Err(ParseError::Empty);
    }

    if body.starts_with('{') || body.starts_with('[') {
        if let Ok(doc) = serde_json::from_str::<Value>(body) {
            if xray_json::looks_like(&doc) {
                let (mut nodes, warnings) = xray_json::parse(&doc);
                return finish(Format::Xray, &mut nodes, warnings);
            }
            if doc.get("outbounds").is_some() {
                let (mut nodes, warnings) = singbox::parse_outbounds(&doc);
                return finish(Format::SingBox, &mut nodes, warnings);
            }
        }
    }

    if let Some((node, warnings)) = wireguard_conf(body) {
        return finish(Format::WireGuard, &mut vec![node], warnings);
    }
    if looks_like_clash(body) {
        let doc: Value = serde_yaml_ng::from_str(body).map_err(|e| ParseError::Clash(e.to_string()))?;
        let mut nodes = Vec::new();
        let mut warnings = Vec::new();
        for p in doc.get("proxies").and_then(Value::as_array).into_iter().flatten() {
            match clash::from_clash(p) {
                Ok(n) => nodes.push(n),
                Err(e) => warnings.push(e),
            }
        }
        // A Clash config may rely entirely on proxy-providers; that is still valid.
        if nodes.is_empty() && doc.get("proxy-providers").is_none() {
            return Err(ParseError::Empty);
        }
        return Ok(Parsed { format: Format::Clash, nodes, warnings });
    }

    let text = if body.contains("://") {
        body.to_string()
    } else {
        links::b64_decode(body).and_then(|b| String::from_utf8(b).ok()).unwrap_or_else(|| body.to_string())
    };
    let mut nodes = Vec::new();
    let mut warnings = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        match links::parse_link(line) {
            Ok(n) => nodes.push(n),
            Err(e) => warnings.push(format!("{}: {e}", truncate(line, 48))),
        }
    }
    finish(Format::Links, &mut nodes, warnings)
}

fn finish(format: Format, nodes: &mut Vec<Node>, warnings: Vec<String>) -> Result<Parsed, ParseError> {
    if nodes.is_empty() {
        return Err(ParseError::Empty);
    }
    dedupe_names(nodes);
    Ok(Parsed { format, nodes: std::mem::take(nodes), warnings })
}

/// A WireGuard `.conf`: `[Interface]` with the key and addresses, `[Peer]` with the
/// server (the first one). `None` when `body` isn't one.
fn wireguard_conf(body: &str) -> Option<(Node, Vec<String>)> {
    if !body.lines().any(|l| l.trim().eq_ignore_ascii_case("[interface]")) {
        return None;
    }
    let (mut section, mut peers) = (String::new(), 0);
    let mut get: std::collections::HashMap<(String, String), String> = Default::default();
    let mut warnings = Vec::new();
    for line in body.lines().map(|l| l.split('#').next().unwrap_or_default().trim()).filter(|l| !l.is_empty()) {
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).trim().to_ascii_lowercase();
            if section == "peer" {
                peers += 1;
            }
            continue;
        }
        if section == "peer" && peers > 1 {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            get.insert((section.clone(), k.trim().to_ascii_lowercase()), v.trim().to_string());
        }
    }
    let field = |sec: &str, k: &str| get.get(&(sec.to_string(), k.to_string())).cloned().filter(|v| !v.is_empty());
    let endpoint = field("peer", "endpoint")?;
    let (host, port) = endpoint.rsplit_once(':')?;
    if peers > 1 {
        warnings.push("Only the first [Peer] of the WireGuard config is used".to_string());
    }
    // AmneziaWG's extra settings, which plain WireGuard servers don't take.
    if ["jc", "jmin", "jmax", "s1", "s2", "h1", "h2", "h3", "h4"].iter().any(|k| field("interface", k).is_some()) {
        warnings.push("The AmneziaWG settings (Jc, S1, H1, ...) aren't used".to_string());
    }
    let reserved = field("peer", "reserved")
        .map(|r| r.trim_matches(['[', ']']).split(',').filter_map(|x| x.trim().parse::<u8>().ok()).collect::<Vec<u8>>())
        .filter(|r| r.len() == 3)
        .unwrap_or_default();
    let addresses: Vec<String> = field("interface", "address").map(|a| a.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let host = host.trim_matches(['[', ']']).to_string();
    let node = Node {
        name: if host.contains("cloudflareclient.com") || host.starts_with("162.159.") { "Cloudflare WARP".into() } else { format!("WireGuard {host}") },
        server: host,
        port: port.parse().ok()?,
        protocol: crate::model::Protocol::Wireguard {
            private_key: field("interface", "privatekey")?,
            public_key: field("peer", "publickey")?,
            pre_shared_key: field("peer", "presharedkey"),
            addresses: addresses.into_iter().map(|a| if a.contains('/') { a } else if a.contains(':') { format!("{a}/128") } else { format!("{a}/32") }).collect(),
            mtu: field("interface", "mtu").and_then(|m| m.parse().ok()),
            reserved,
        },
        transport: crate::model::Transport::Tcp,
        tls: None,
        udp: true,
        mux: None,
    };
    Some((node, warnings))
}

fn looks_like_clash(body: &str) -> bool {
    body.lines().any(|l| l.starts_with("proxies:") || l.starts_with("proxy-providers:") || l.starts_with("proxy-groups:"))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// The `subscription-userinfo` response header: `upload=1; download=2; total=3; expire=4`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    /// Unix seconds; `None` when the plan does not expire.
    pub expire: Option<i64>,
}

pub fn parse_userinfo(header: &str) -> UserInfo {
    let mut u = UserInfo::default();
    for part in header.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let v = v.trim();
        // Some panels send floats such as `total=1.073741824E10`.
        let n = v.parse::<u64>().ok().or_else(|| v.parse::<f64>().ok().map(|f| f as u64));
        match k.trim() {
            "upload" => u.upload = n.unwrap_or(0),
            "download" => u.download = n.unwrap_or(0),
            "total" => u.total = n.unwrap_or(0),
            "expire" => u.expire = n.filter(|e| *e > 0).map(|e| e as i64),
            _ => {}
        }
    }
    u
}

/// `profile-title` may be plain text or `base64:...`.
pub fn decode_title(header: &str) -> String {
    match header.strip_prefix("base64:") {
        Some(b) => links::b64_decode(b).and_then(|v| String::from_utf8(v).ok()).unwrap_or_else(|| header.to_string()),
        None => header.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn base64_link_list_with_bad_line() {
        let raw = "vless://id@a.example:443?security=tls#One\ntrojan://pw@b.example:443#One\nnonsense\n";
        let body = base64::engine::general_purpose::STANDARD.encode(raw);
        let p = parse(&body).unwrap();
        assert_eq!(p.format, Format::Links);
        assert_eq!(p.nodes.len(), 2);
        assert_eq!(p.nodes[1].name, "One 2");
        assert_eq!(p.warnings.len(), 1);
    }

    #[test]
    fn clash_yaml() {
        let y = "mixed-port: 7890\nproxies:\n  - {name: A, type: trojan, server: a.example, port: 443, password: x}\n  - {name: B, type: snell, server: b, port: 1, psk: y}\nproxy-groups:\n  - {name: P, type: select, proxies: [A, B]}\nrules:\n  - MATCH,P\n";
        let p = parse(y).unwrap();
        assert_eq!(p.format, Format::Clash);
        assert_eq!(p.nodes.len(), 2);
    }

    #[test]
    fn singbox_json() {
        let j = r#"{"outbounds":[{"type":"selector","tag":"s"},{"type":"vless","tag":"V","server":"a","server_port":443,"uuid":"u","tls":{"enabled":true,"server_name":"x","reality":{"enabled":true,"public_key":"k","short_id":"1"}}},{"type":"direct","tag":"d"}]}"#;
        let p = parse(j).unwrap();
        assert_eq!(p.format, Format::SingBox);
        assert_eq!(p.nodes.len(), 1);
        assert!(p.nodes[0].tls.as_ref().unwrap().reality.is_some());
    }

    #[test]
    fn userinfo_and_title() {
        let u = parse_userinfo("upload=455727941; download=6174315083; total=1.073741824E11; expire=1798675200");
        assert_eq!(u.total, 107374182400);
        assert_eq!(u.expire, Some(1798675200));
        assert_eq!(decode_title("base64:0J/RgNC40LLQtdGC"), "Привет");
        assert_eq!(parse_userinfo("upload=1; expire=0").expire, None);
    }

    /// A WARP profile as wgcf writes it, and an Xray outbound Kova doesn't model.
    #[test]
    fn wireguard_conf_and_masque() {
        let conf = "[Interface]\nPrivateKey = cHJpdmF0ZWtleXByaXZhdGVrZXlwcml2YXRla2V5MDA9\nAddress = 172.16.0.2/32, 2606:4700:110:8a36::2/128\nDNS = 1.1.1.1\nMTU = 1280\n\n[Peer]\nPublicKey = bmXOC+F1FxEMF9dyiK2H5/1SUtzH0JuVo51h2wPfgyo=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = engage.cloudflareclient.com:2408\n";
        let p = parse(conf).unwrap();
        assert_eq!(p.format, Format::WireGuard);
        let n = &p.nodes[0];
        assert_eq!((n.name.as_str(), n.server.as_str(), n.port), ("Cloudflare WARP", "engage.cloudflareclient.com", 2408));
        assert!(matches!(&n.protocol, crate::model::Protocol::Wireguard { addresses, mtu: Some(1280), .. } if addresses.len() == 2));
        assert_eq!(crate::clash::to_clash(n)["type"], "wireguard");

        let masque = r#"{"remarks":"WARP over MASQUE","outbounds":[{"tag":"proxy","protocol":"masque","settings":{"address":"162.159.198.2","port":443},"streamSettings":{"method":"masque","security":"tls"}}]}"#;
        let p = parse(masque).unwrap();
        let n = &p.nodes[0];
        assert_eq!((n.server.as_str(), n.port, n.protocol_label().as_str()), ("162.159.198.2", 443, "MASQUE"));
        assert!(!crate::compat::supports(crate::CoreKind::Mihomo, n) && crate::compat::supports(crate::CoreKind::Xray, n));
        let o = crate::gen_xray::outbound(n).unwrap();
        assert_eq!((o["tag"].as_str(), o["protocol"].as_str()), (Some("WARP over MASQUE"), Some("masque")));
    }
}
