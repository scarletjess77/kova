//! The facts about one server that the interface's details view shows. Built from the
//! parsed [`Node`], so every profile format looks the same. Secrets are flagged so the
//! interface can hide them until asked.

use kova_core::links::to_link;
use kova_core::model::{Mux, Node, Protocol, Tls, Transport};
use serde::Serialize;
use serde_json::Value;

/// The labels, one per concept. The interface translates a label by looking it up, so
/// they are written once, here.
mod label {
    pub const NAME: &str = "Name";
    pub const PROTOCOL: &str = "Protocol";
    pub const SERVER: &str = "Server";
    pub const PORT: &str = "Port";
    pub const TRANSPORT: &str = "Transport";
    pub const PATH: &str = "Path";
    pub const HOST: &str = "Host header";
    pub const SERVICE_NAME: &str = "Service name";
    pub const MODE: &str = "Mode";
    pub const EXTRA: &str = "Extra settings";
    pub const SECURITY: &str = "Security";
    pub const SERVER_NAME: &str = "Server name (SNI)";
    pub const ALPN: &str = "ALPN";
    pub const FINGERPRINT: &str = "Fingerprint";
    pub const ALLOW_INSECURE: &str = "Allow insecure";
    pub const CERT_PIN: &str = "Certificate pin";
    pub const ECH: &str = "Encrypted Client Hello";
    pub const REALITY_KEY: &str = "Reality public key";
    pub const REALITY_SHORT_ID: &str = "Reality short ID";
    pub const REALITY_SPIDER_X: &str = "Reality spider path";
    pub const UDP: &str = "UDP";
    pub const MUX: &str = "Multiplexing";
    pub const UUID: &str = "UUID";
    pub const FLOW: &str = "Flow";
    pub const ENCRYPTION: &str = "Encryption";
    pub const ALTER_ID: &str = "Alter ID";
    pub const CIPHER: &str = "Cipher";
    pub const METHOD: &str = "Method";
    pub const USERNAME: &str = "Username";
    pub const PASSWORD: &str = "Password";
    pub const PADDING: &str = "Padding";
    pub const PLUGIN: &str = "Plugin";
    pub const PLUGIN_OPTION: &str = "Plugin option";
    pub const OBFS: &str = "Obfuscation";
    pub const OBFS_PASSWORD: &str = "Obfuscation password";
    pub const PORT_HOPPING: &str = "Port hopping";
    pub const UPLOAD: &str = "Upload limit";
    pub const DOWNLOAD: &str = "Download limit";
    pub const CONGESTION: &str = "Congestion control";
    pub const UDP_RELAY: &str = "UDP relay mode";
    pub const PRIVATE_KEY: &str = "Private key";
    pub const PUBLIC_KEY: &str = "Public key";
    pub const PRE_SHARED_KEY: &str = "Pre-shared key";
    pub const ADDRESSES: &str = "Addresses";
    pub const MTU: &str = "MTU";
    pub const RESERVED: &str = "Reserved bytes";
    pub const KIND: &str = "Type";
    pub const SHARE_LINK: &str = "Share link";
}

/// What stands in for a secret value in the raw view.
const MASK: &str = "•••";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Field {
    pub label: String,
    pub value: String,
    /// Passwords, ids and keys: shown only on request.
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeDetails {
    pub name: String,
    pub fields: Vec<Field>,
    /// The server's settings as JSON, for kinds Kova doesn't model; secrets masked.
    pub raw: Option<String>,
}

#[derive(Default)]
struct Fields(Vec<Field>);

impl Fields {
    fn add(&mut self, label: &str, value: impl Into<String>, secret: bool) {
        self.0.push(Field { label: label.into(), value: value.into(), secret });
    }
    fn plain(&mut self, label: &str, value: impl Into<String>) {
        self.add(label, value, false);
    }
    fn secret(&mut self, label: &str, value: impl Into<String>) {
        self.add(label, value, true);
    }
    /// A field that only exists when the server sets it (and isn't blank).
    fn opt(&mut self, label: &str, value: Option<&str>) {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            self.plain(label, v);
        }
    }
    fn opt_secret(&mut self, label: &str, value: Option<&str>) {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            self.secret(label, v);
        }
    }
}

fn yes_no(b: bool) -> &'static str {
    if b { "Yes" } else { "No" }
}

/// Every detail Kova knows about `n`.
pub fn details(n: &Node) -> NodeDetails {
    let mut f = Fields::default();
    f.plain(label::NAME, n.name.clone());
    f.plain(label::PROTOCOL, n.protocol_label());
    f.plain(label::SERVER, n.server.clone());
    f.plain(label::PORT, n.port.to_string());
    transport(&mut f, &n.transport);
    if let Some(tls) = &n.tls {
        security(&mut f, tls);
    }
    f.plain(label::UDP, yes_no(n.udp));
    if let Some(m) = &n.mux {
        f.plain(label::MUX, mux(m));
    }
    let mut raw = None;
    match &n.protocol {
        Protocol::Vless { uuid, flow, encryption } => {
            f.secret(label::UUID, uuid.clone());
            f.opt(label::FLOW, flow.as_deref());
            f.opt(label::ENCRYPTION, encryption.as_deref());
        }
        Protocol::Vmess { uuid, alter_id, cipher } => {
            f.secret(label::UUID, uuid.clone());
            f.plain(label::ALTER_ID, alter_id.to_string());
            f.plain(label::CIPHER, cipher.clone());
        }
        Protocol::Trojan { password } | Protocol::Anytls { password } => f.secret(label::PASSWORD, password.clone()),
        Protocol::Shadowsocks { method, password, plugin } => {
            f.plain(label::METHOD, method.clone());
            f.secret(label::PASSWORD, password.clone());
            if let Some(p) = plugin {
                f.plain(label::PLUGIN, p.name.clone());
                for (k, v) in &p.opts {
                    let text = match v {
                        Value::String(s) => s.clone(),
                        v => v.to_string(),
                    };
                    f.add(label::PLUGIN_OPTION, format!("{k} = {text}"), secret_key(k));
                }
            }
        }
        Protocol::Hysteria2 { password, obfs, obfs_password, ports, up_mbps, down_mbps } => {
            f.secret(label::PASSWORD, password.clone());
            f.opt(label::OBFS, obfs.as_deref());
            f.opt_secret(label::OBFS_PASSWORD, obfs_password.as_deref());
            f.opt(label::PORT_HOPPING, ports.as_deref());
            if let Some(u) = up_mbps {
                f.plain(label::UPLOAD, format!("{u} Mbps"));
            }
            if let Some(d) = down_mbps {
                f.plain(label::DOWNLOAD, format!("{d} Mbps"));
            }
        }
        Protocol::Tuic { uuid, password, congestion_control, udp_relay_mode } => {
            f.secret(label::UUID, uuid.clone());
            f.secret(label::PASSWORD, password.clone());
            f.opt(label::CONGESTION, congestion_control.as_deref());
            f.opt(label::UDP_RELAY, udp_relay_mode.as_deref());
        }
        Protocol::Wireguard { private_key, public_key, pre_shared_key, addresses, mtu, reserved } => {
            f.secret(label::PRIVATE_KEY, private_key.clone());
            f.plain(label::PUBLIC_KEY, public_key.clone());
            f.opt_secret(label::PRE_SHARED_KEY, pre_shared_key.as_deref());
            if !addresses.is_empty() {
                f.plain(label::ADDRESSES, addresses.join(", "));
            }
            if let Some(m) = mtu {
                f.plain(label::MTU, m.to_string());
            }
            if !reserved.is_empty() {
                f.plain(label::RESERVED, reserved.iter().map(u8::to_string).collect::<Vec<_>>().join(", "));
            }
        }
        Protocol::Socks { username, password } | Protocol::Http { username, password } => {
            f.opt(label::USERNAME, username.as_deref());
            f.opt_secret(label::PASSWORD, password.as_deref());
        }
        Protocol::Naive { username, password, padding } => {
            f.plain(label::USERNAME, username.clone());
            f.secret(label::PASSWORD, password.clone());
            f.plain(label::PADDING, yes_no(*padding));
        }
        Protocol::Other { kind, raw: r } | Protocol::XrayOutbound { kind, raw: r } => {
            f.plain(label::KIND, kind.clone());
            let mut shown = r.clone();
            mask(&mut shown);
            raw = serde_json::to_string_pretty(&shown).ok();
        }
    }
    if let Some(link) = to_link(n) {
        f.secret(label::SHARE_LINK, link);
    }
    NodeDetails { name: n.name.clone(), fields: f.0, raw }
}

fn transport(f: &mut Fields, t: &Transport) {
    f.plain(label::TRANSPORT, t.label());
    match t {
        Transport::Tcp => {}
        Transport::Ws { path, host } | Transport::H2 { path, host } | Transport::HttpUpgrade { path, host } => {
            f.opt(label::PATH, Some(path));
            f.opt(label::HOST, host.as_deref());
        }
        Transport::Grpc { service_name } => f.opt(label::SERVICE_NAME, Some(service_name)),
        Transport::Xhttp { path, host, mode, extra } => {
            f.opt(label::PATH, Some(path));
            f.opt(label::HOST, host.as_deref());
            f.opt(label::MODE, mode.as_deref());
            if let Some(e) = extra {
                let mut shown = e.clone();
                mask(&mut shown);
                f.plain(label::EXTRA, shown.to_string());
            }
        }
    }
}

fn security(f: &mut Fields, tls: &Tls) {
    f.plain(label::SECURITY, if tls.reality.is_some() { "Reality" } else { "TLS" });
    f.opt(label::SERVER_NAME, tls.sni.as_deref());
    if !tls.alpn.is_empty() {
        f.plain(label::ALPN, tls.alpn.join(", "));
    }
    f.opt(label::FINGERPRINT, tls.fingerprint.as_deref());
    f.plain(label::ALLOW_INSECURE, yes_no(tls.insecure));
    f.opt(label::CERT_PIN, tls.pin.as_deref());
    if let Some(e) = &tls.ech {
        f.plain(label::ECH, e.query.clone().or_else(|| e.config.clone()).unwrap_or_default());
    }
    if let Some(r) = &tls.reality {
        f.plain(label::REALITY_KEY, r.public_key.clone());
        f.opt(label::REALITY_SHORT_ID, r.short_id.as_deref());
        f.opt(label::REALITY_SPIDER_X, r.spider_x.as_deref());
    }
}

fn mux(m: &Mux) -> String {
    match m {
        Mux::Xray { concurrency, .. } => match concurrency {
            Some(c) => format!("Xray Mux, {c} connections"),
            None => "Xray Mux".into(),
        },
        Mux::Sing { protocol, padding, .. } => {
            let mut s = format!("sing-mux {}", protocol.as_deref().unwrap_or("")).trim().to_string();
            if *padding {
                s.push_str(", padding");
            }
            s
        }
    }
}

/// Whether a setting called `key` holds a secret.
fn secret_key(key: &str) -> bool {
    let k: String = key.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).collect::<String>().to_lowercase();
    ["id", "key", "pass", "seed", "protocolparam", "obfsparam"].contains(&k.as_str()) || ["uuid", "password", "privatekey", "presharedkey", "psk", "token", "auth", "secret"].iter().any(|s| k.contains(s))
}

/// [`secret_key`] for a JSON member: `obfs` is a secret (a password in some cores) when
/// it is a string, while an object of that name only holds settings.
fn secret_member(key: &str, value: &Value) -> bool {
    secret_key(key) || (value.is_string() && key.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).collect::<String>().eq_ignore_ascii_case("obfs"))
}

/// Replace the secret values in `v` (a copy made for display) with [`MASK`].
fn mask(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if secret_member(k, x) && !x.is_null() {
                    *x = Value::String(MASK.into());
                } else {
                    mask(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(mask),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kova_core::links::parse_link;
    use kova_core::model::SsPlugin;

    fn get<'a>(d: &'a NodeDetails, label: &str) -> Vec<&'a Field> {
        d.fields.iter().filter(|f| f.label == label).collect()
    }

    #[test]
    fn vless_reality_shows_its_parts_and_hides_the_secrets() {
        let n = parse_link("vless://11111111-1111-1111-1111-111111111111@a.example:443?security=reality&pbk=PUBKEY&sid=ab12&sni=www.example.com&fp=firefox&flow=xtls-rprx-vision#R").unwrap();
        let d = details(&n);
        assert_eq!(d.name, "R");
        assert!(d.raw.is_none());
        let one = |l: &str| -> &Field {
            let v = get(&d, l);
            assert_eq!(v.len(), 1, "{l}");
            v[0]
        };
        assert_eq!(one("Protocol").value, "VLESS");
        assert_eq!((one("Server").value.as_str(), one("Port").value.as_str()), ("a.example", "443"));
        assert_eq!(one("Security").value, "Reality");
        assert_eq!(one("Server name (SNI)").value, "www.example.com");
        assert_eq!(one("Fingerprint").value, "firefox");
        assert_eq!((one("Reality public key").value.as_str(), one("Reality public key").secret), ("PUBKEY", false));
        assert_eq!(one("Reality short ID").value, "ab12");
        assert_eq!(one("Flow").value, "xtls-rprx-vision");
        assert_eq!(one("UDP").value, "Yes");
        let uuid = one("UUID");
        assert!(uuid.secret && uuid.value.starts_with("11111111"));
        let link = one("Share link");
        assert!(link.secret && link.value.starts_with("vless://"));
        for f in &d.fields {
            assert_eq!(f.secret, matches!(f.label.as_str(), "UUID" | "Share link"), "{}", f.label);
        }
    }

    #[test]
    fn shadowsocks_plugin_options_are_listed() {
        let n = Node {
            name: "SS".into(),
            server: "s.example".into(),
            port: 8388,
            protocol: Protocol::Shadowsocks {
                method: "2022-blake3-aes-128-gcm".into(),
                password: "hunter2".into(),
                plugin: Some(SsPlugin {
                    name: "shadow-tls".into(),
                    opts: [("host".to_string(), Value::String("cdn.example".into())), ("password".to_string(), Value::String("tls-pw".into()))].into_iter().collect(),
                }),
            },
            transport: Transport::Tcp,
            tls: None,
            udp: false,
            mux: None,
        };
        let d = details(&n);
        let field = |l: &str| get(&d, l).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(field("Method")[0].value, "2022-blake3-aes-128-gcm");
        assert!(field("Password")[0].secret);
        assert_eq!(field("Plugin")[0].value, "shadow-tls");
        let opts = field("Plugin option");
        assert_eq!(opts.len(), 2);
        assert_eq!((opts[0].value.as_str(), opts[0].secret), ("host = cdn.example", false));
        assert_eq!((opts[1].value.as_str(), opts[1].secret), ("password = tls-pw", true));
        assert_eq!(field("UDP")[0].value, "No");
    }

    #[test]
    fn wider_secret_keys_are_masked() {
        let raw = serde_json::json!({ "key": "k1", "pass": "p1", "seed": "s1", "protocol_param": "pp", "obfs-param": "op", "obfs": "tls1", "obfs-opts": { "obfs": { "mode": "x" } }, "keyword": "visible", "passive": "visible2" });
        let mut shown = raw;
        mask(&mut shown);
        let text = shown.to_string();
        for leaked in ["k1", "p1", "s1", "\"pp\"", "\"op\"", "tls1"] {
            assert!(!text.contains(leaked), "{leaked} in {text}");
        }
        assert!(text.contains("visible\"") && text.contains("visible2") && text.contains("\"mode\":\"x\""), "{text}");
    }

    #[test]
    fn xhttp_extra_is_masked() {
        let n = Node {
            name: "X".into(),
            server: "x.example".into(),
            port: 443,
            protocol: Protocol::Trojan { password: "pw".into() },
            transport: Transport::Xhttp { path: "/".into(), host: None, mode: None, extra: Some(serde_json::json!({ "downloadSettings": { "key": "dl-secret" }, "xPaddingBytes": "100-1000" })) },
            tls: None,
            udp: true,
            mux: None,
        };
        let d = details(&n);
        let extra = &get(&d, "Extra settings")[0].value;
        assert!(!extra.contains("dl-secret") && extra.contains(MASK) && extra.contains("100-1000"), "{extra}");
    }

    #[test]
    fn unmodelled_kinds_show_masked_json() {
        let raw = serde_json::json!({ "type": "snell", "psk": "topsecret", "version": 3, "obfs-opts": { "mode": "tls", "Password": "x" }, "list": [{ "uuid": "u" }], "id": "abc", "server-id": 7 });
        let n = Node { name: "O".into(), server: "o.example".into(), port: 1, protocol: Protocol::Other { kind: "snell".into(), raw }, transport: Transport::Tcp, tls: None, udp: true, mux: None };
        let d = details(&n);
        assert_eq!(get(&d, "Type")[0].value, "snell");
        let shown = d.raw.expect("raw JSON");
        for leaked in ["topsecret", "\"x\"", "\"u\"", "\"abc\""] {
            assert!(!shown.contains(leaked), "{leaked} in {shown}");
        }
        assert_eq!(shown.matches(MASK).count(), 4, "{shown}");
        assert!(shown.contains("\"version\": 3") && shown.contains("\"mode\": \"tls\"") && shown.contains("\"server-id\": 7"), "{shown}");
        assert!(d.fields.iter().all(|f| !f.value.contains("topsecret")));
    }
}
