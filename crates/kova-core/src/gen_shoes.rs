//! Shoes config: a helper core, like the Xray sidecar, for servers neither Mihomo nor
//! Xray can run (NaiveProxy). One local SOCKS port per server, with Kova's login, which
//! Mihomo dials as an ordinary SOCKS server; Shoes carries it to the real server.

use serde_json::{Value, json};

use crate::model::{Node, Protocol};
use crate::options::{LOCAL_USER, RuntimeOptions};

/// `host:port`, with IPv6 in brackets.
fn address(n: &Node) -> String {
    if n.server.contains(':') { format!("[{}]:{}", n.server, n.port) } else { format!("{}:{}", n.server, n.port) }
}

/// How Shoes reaches `n`, or `None` if Shoes can't run it.
fn client(n: &Node) -> Option<Value> {
    match &n.protocol {
        Protocol::Naive { username, password, padding } => {
            let tls = n.tls.clone().unwrap_or_default();
            Some(json!({
                "address": address(n),
                "protocol": {
                    "type": "tls",
                    "verify": !tls.insecure,
                    "sni_hostname": tls.sni.unwrap_or_else(|| n.server.clone()),
                    // NaiveProxy is HTTP/2 CONNECT.
                    "alpn_protocols": ["h2"],
                    "protocol": { "type": "naiveproxy", "username": username, "password": password, "padding": padding },
                },
            }))
        }
        _ => None,
    }
}

/// Whether Shoes runs `n` (see `compat`).
pub fn supports(n: &Node) -> bool {
    client(n).is_some()
}

/// The config: a local SOCKS listener on each node's port that sends everything to it.
pub fn config(bridged: &[(&Node, u16)], opts: &RuntimeOptions) -> String {
    let listeners: Vec<Value> = bridged
        .iter()
        .filter_map(|(n, port)| {
            let mut socks = json!({ "type": "socks", "udp_enabled": true });
            // Only Mihomo uses these ports; with a login no other local program can.
            if !opts.local_password.is_empty() {
                socks["username"] = LOCAL_USER.into();
                socks["password"] = opts.local_password.clone().into();
            }
            Some(json!({
                "address": format!("127.0.0.1:{port}"),
                "protocol": socks,
                // Any destination, names and IPv6 included.
                "rules": [{ "masks": "0.0.0.0/0", "action": "allow", "client_chain": client(n)? }],
            }))
        })
        .collect();
    serde_yaml_ng::to_string(&listeners).expect("json to yaml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::parse_link;

    #[test]
    fn a_naive_server_gets_a_local_socks_port() {
        let n = parse_link("naive+https://user:p%40ss@naive.example.com:443?padding=true#Naive").unwrap();
        let o = RuntimeOptions { local_password: "pw".into(), ..Default::default() };
        let doc: Value = serde_yaml_ng::from_str(&config(&[(&n, 17200)], &o)).unwrap();
        let l = &doc[0];
        assert_eq!(l["address"], "127.0.0.1:17200");
        assert_eq!((l["protocol"]["type"].as_str(), l["protocol"]["username"].as_str()), (Some("socks"), Some(LOCAL_USER)));
        let chain = &l["rules"][0]["client_chain"];
        assert_eq!(l["rules"][0]["masks"], "0.0.0.0/0");
        assert_eq!(chain["address"], "naive.example.com:443");
        assert_eq!(chain["protocol"]["type"], "tls");
        assert_eq!(chain["protocol"]["sni_hostname"], "naive.example.com");
        assert_eq!(chain["protocol"]["protocol"], json!({ "type": "naiveproxy", "username": "user", "password": "p@ss", "padding": true }));
        assert!(!supports(&parse_link("trojan://pw@a.example:443#T").unwrap()));
    }
}
