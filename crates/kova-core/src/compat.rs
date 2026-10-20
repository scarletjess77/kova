//! Which core can run which node, and the plan for running a profile.
//!
//! Keep this table in sync with upstream releases: it is the only place that encodes
//! core capabilities.

use serde::{Deserialize, Serialize};

use crate::model::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreKind {
    Mihomo,
    Xray,
}

impl CoreKind {
    pub fn label(self) -> &'static str {
        match self {
            CoreKind::Mihomo => "Mihomo",
            CoreKind::Xray => "Xray",
        }
    }
}

/// The user's choice for a profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CorePreference {
    /// Mihomo in front; nodes only Xray can run are bridged through an Xray sidecar.
    #[default]
    Auto,
    Mihomo,
    Xray,
}

/// `None` if `core` can run `node`, otherwise a short reason for the UI.
pub fn unsupported(core: CoreKind, node: &Node) -> Option<&'static str> {
    match core {
        CoreKind::Mihomo => {
            if matches!(node.transport, Transport::Xhttp { .. }) {
                return Some("XHTTP transport needs Xray");
            }
            if let Protocol::Vless { encryption: Some(e), .. } = &node.protocol {
                if e != "none" {
                    return Some("VLESS encryption needs Xray");
                }
            }
            None
        }
        CoreKind::Xray => match &node.protocol {
            Protocol::Hysteria2 { .. } => Some("Hysteria2 needs Mihomo"),
            Protocol::Tuic { .. } => Some("TUIC needs Mihomo"),
            Protocol::Anytls { .. } => Some("AnyTLS needs Mihomo"),
            Protocol::Other { .. } => Some("this protocol needs Mihomo"),
            Protocol::Shadowsocks { plugin: Some(_), .. } => Some("Shadowsocks plugins need Mihomo"),
            _ if matches!(node.transport, Transport::H2 { .. }) => Some("HTTP/2 transport was removed from Xray"),
            _ => None,
        },
    }
}

pub fn supports(core: CoreKind, node: &Node) -> bool {
    unsupported(core, node).is_none()
}

/// How a profile's nodes will be run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The core that owns the ports, rules and TUN.
    pub front: CoreKind,
    /// Node names that Mihomo reaches through a local Xray SOCKS bridge.
    pub bridged: Vec<String>,
    /// Nodes that no selected core can run, with the reason.
    pub dropped: Vec<(String, String)>,
}

impl Plan {
    pub fn needs_sidecar(&self) -> bool {
        self.front == CoreKind::Mihomo && !self.bridged.is_empty()
    }
}

pub fn plan(nodes: &[Node], pref: CorePreference) -> Plan {
    let mut p = Plan { front: CoreKind::Mihomo, bridged: Vec::new(), dropped: Vec::new() };
    match pref {
        CorePreference::Xray => {
            p.front = CoreKind::Xray;
            for n in nodes {
                if let Some(r) = unsupported(CoreKind::Xray, n) {
                    p.dropped.push((n.name.clone(), r.into()));
                }
            }
        }
        CorePreference::Mihomo | CorePreference::Auto => {
            for n in nodes {
                if let Some(r) = unsupported(CoreKind::Mihomo, n) {
                    if pref == CorePreference::Auto && supports(CoreKind::Xray, n) {
                        p.bridged.push(n.name.clone());
                    } else {
                        p.dropped.push((n.name.clone(), r.into()));
                    }
                }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::parse_link;

    #[test]
    fn auto_bridges_xray_only_nodes() {
        let nodes = vec![
            parse_link("vless://id@a:443?security=reality&pbk=k&type=xhttp&path=/x#xh").unwrap(),
            parse_link("hy2://pw@b:443#hy").unwrap(),
            parse_link("trojan://pw@c:443#tr").unwrap(),
        ];
        let p = plan(&nodes, CorePreference::Auto);
        assert_eq!(p.front, CoreKind::Mihomo);
        assert_eq!(p.bridged, vec!["xh"]);
        assert!(p.dropped.is_empty());

        let x = plan(&nodes, CorePreference::Xray);
        assert_eq!(x.dropped.len(), 1);
        assert_eq!(x.dropped[0].0, "hy");

        let m = plan(&nodes, CorePreference::Mihomo);
        assert_eq!(m.dropped[0].0, "xh");
    }
}
