//! Mihomo config generation.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use serde_yaml_ng::{Mapping, Value as Yaml};

use crate::clash::to_clash;
use crate::compat::Plan;
use crate::model::Node;
use crate::options::*;

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("the profile is not a valid YAML mapping: {0}")]
    Yaml(String),
}

const GEOX_BASE: &str = "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest";

fn geox_url(opts: &RuntimeOptions) -> Value {
    let (geoip, geosite) = opts.geodata.urls();
    json!({
        "geoip": geoip,
        "geosite": geosite,
        "mmdb": format!("{GEOX_BASE}/country.mmdb"),
        "asn": format!("{GEOX_BASE}/GeoLite2-ASN.mmdb"),
    })
}

/// Settings every Mihomo config gets, whatever the profile says.
fn base(opts: &RuntimeOptions) -> Value {
    json!({
        "mixed-port": opts.mixed_port,
        "allow-lan": opts.allow_lan,
        "bind-address": if opts.allow_lan { "*" } else { "127.0.0.1" },
        "mode": opts.mode,
        "log-level": opts.log_level,
        "ipv6": opts.ipv6,
        "unified-delay": true,
        "tcp-concurrent": true,
        // Android doesn't let an app see other apps' connections.
        "find-process-mode": if opts.tun_fd.is_some() { "off" } else { "always" },
        "external-controller": format!("127.0.0.1:{}", opts.controller_port),
        "secret": opts.secret,
        "geodata-mode": true,
        "geo-auto-update": true,
        "geo-update-interval": 24,
        "geox-url": geox_url(opts),
        "profile": { "store-selected": true, "store-fake-ip": true },
    })
}

fn sniffer() -> Value {
    json!({
        "enable": true,
        "parse-pure-ip": true,
        "sniff": {
            "HTTP": { "ports": [80, "8080-8880"], "override-destination": true },
            "TLS": { "ports": [443, 8443] },
            "QUIC": { "ports": [443, 8443] },
        },
        "skip-domain": ["Mijia Cloud", "+.push.apple.com"],
    })
}

fn dns(opts: &RuntimeOptions) -> Value {
    let d = &opts.dns;
    let mut policy = serde_json::Map::new();
    let geosites: Vec<&str> = opts.presets.iter().flat_map(|p| p.direct_geosites().iter().copied()).collect();
    if !geosites.is_empty() {
        policy.insert(format!("geosite:{}", geosites.join(",")), json!(d.direct_nameservers));
    }
    let mut v = json!({
        "enable": true,
        "ipv6": opts.ipv6,
        "enhanced-mode": if d.fake_ip { "fake-ip" } else { "redir-host" },
        "fake-ip-range": "198.18.0.1/16",
        "fake-ip-filter": [
            "*.lan", "+.local", "+.localhost", "+.home.arpa",
            "+.msftconnecttest.com", "+.msftncsi.com", "time.*.com", "+.ntp.org", "+.pool.ntp.org",
            "+.stun.*.*", "+.stun.*.*.*", "localhost.ptlogin2.qq.com",
            "+.xboxlive.com", "+.nintendo.net", "+.playstation.net",
        ],
        "default-nameserver": ["1.1.1.1", "8.8.8.8", "77.88.8.8"],
        "nameserver": d.nameservers,
        "proxy-server-nameserver": d.direct_nameservers,
        "direct-nameserver": d.direct_nameservers,
    });
    if !policy.is_empty() {
        v["nameserver-policy"] = Value::Object(policy);
    }
    v
}

fn tun(opts: &RuntimeOptions) -> Value {
    let t = &opts.tun;
    let mut v = json!({
        "enable": t.enabled,
        "stack": t.stack,
        "auto-route": t.auto_route,
        "auto-redirect": cfg!(target_os = "linux") && t.auto_route,
        "auto-detect-interface": true,
        "strict-route": t.strict_route,
        "dns-hijack": ["any:53", "tcp://any:53"],
        "mtu": t.mtu,
    });
    if !opts.tun_exclude.is_empty() {
        v["route-exclude-address"] = json!(opts.tun_exclude);
    }
    if let Some(fd) = opts.tun_fd {
        // Android: the system made the interface and set its routes and DNS; Kova's own
        // traffic (the core included) is kept out of it by the VPN itself.
        v["file-descriptor"] = json!(fd);
        v["stack"] = json!("gvisor");
        for k in ["auto-route", "auto-redirect", "auto-detect-interface", "strict-route"] {
            v[k] = json!(false);
        }
    }
    v
}

/// Rules that must come before everything else: traffic of processes that are part of
/// Kova itself goes direct. In TUN mode the Xray bridge's connections to its servers are
/// captured too, and without this they would be routed back into the bridge.
fn bypass_rules(opts: &RuntimeOptions) -> Vec<String> {
    opts.bypass_processes.iter().map(|p| format!("PROCESS-NAME,{p},DIRECT")).collect()
}

/// For the "only blocked sites" preset, the catch-all must be DIRECT whatever the
/// profile's own rules say.
fn blocked_only(opts: &RuntimeOptions) -> bool {
    opts.presets.contains(&Preset::ProxyBlockedRu)
}

/// Smart routing's rule lists and probe ports, added to the config when it is on.
/// Learned sites come right after the user's rules and presets: they were learned
/// from traffic that only the catch-all matched, so they never override a rule.
fn apply_smart(doc: &mut Mapping, opts: &RuntimeOptions, main: &str) -> Vec<String> {
    use crate::smart::{PROVIDER_DIRECT, PROVIDER_DIRECT_IP, PROVIDER_PROXY, PROVIDER_PROXY_IP, PROVIDERS};
    let Some(sm) = &opts.smart else { return Vec::new() };
    let to_yaml = |v: Value| serde_yaml_ng::to_value(v).expect("json to yaml");
    let provider = |name: &str| {
        // Fetched from Kova over loopback and refreshed through the controller whenever
        // a site is learned; `interval: 0` keeps Mihomo from polling on its own.
        let behavior = if name.ends_with("-ip") { "ipcidr" } else { "domain" };
        to_yaml(json!({ "type": "http", "behavior": behavior, "format": "text", "url": format!("{}{name}.txt", sm.lists_url), "path": format!("./{name}.txt"), "interval": 0 }))
    };
    let mut providers = match doc.remove("rule-providers") {
        Some(Yaml::Mapping(m)) => m,
        _ => Mapping::new(),
    };
    for name in PROVIDERS {
        providers.insert(Yaml::String(name.into()), provider(name));
    }
    doc.insert(Yaml::String("rule-providers".into()), Yaml::Mapping(providers));

    let mut listeners = match doc.remove("listeners") {
        Some(Yaml::Sequence(s)) => s,
        _ => Vec::new(),
    };
    for (name, port, proxy) in [("kova-probe-direct", sm.direct_port, "DIRECT"), ("kova-probe-proxy", sm.proxy_port, main)] {
        listeners.push(to_yaml(json!({ "name": name, "type": "mixed", "listen": "127.0.0.1", "port": port, "udp": false, "proxy": proxy })));
    }
    doc.insert(Yaml::String("listeners".into()), Yaml::Sequence(listeners));
    vec![
        format!("RULE-SET,{PROVIDER_DIRECT},DIRECT"),
        format!("RULE-SET,{PROVIDER_PROXY},{main}"),
        // Addresses only match connections that are already to an address: a name is
        // never resolved just to check these.
        format!("RULE-SET,{PROVIDER_DIRECT_IP},DIRECT,no-resolve"),
        format!("RULE-SET,{PROVIDER_PROXY_IP},{main},no-resolve"),
    ]
}

/// Who each rule in the config belongs to, by the name Mihomo logs it under, for smart
/// routing to decide what it may do about a failure. `profile_rules`: an imported
/// profile's own rules may be corrected.
pub fn smart_scope(opts: &RuntimeOptions, profile_rules: bool) -> crate::smart::Scope {
    use crate::smart::{PROVIDERS, rule_key};
    let key = |r: &Rule| rule_key(r.kind.mihomo_name(), &r.value);
    let mut internal: std::collections::HashSet<String> = opts.tail_rules().iter().map(key).collect();
    internal.extend(opts.bypass_processes.iter().map(|p| rule_key("ProcessName", p)));
    internal.extend(PROVIDERS.iter().map(|p| rule_key("RuleSet", p)));
    crate::smart::Scope {
        user_rules: opts.custom_rules.iter().map(key).collect(),
        preset_rules: Preset::ALL.iter().filter(|p| opts.presets.contains(p)).flat_map(|p| p.rules()).map(|r| key(&r)).collect(),
        internal_rules: internal,
        profile_rules,
    }
}

/// Address of the local Xray bridge for each bridged node.
pub type Bridges = HashMap<String, u16>;

/// Config for a profile made of share links: Kova provides groups and rules.
pub fn from_nodes(nodes: &[Node], plan: &Plan, bridges: &Bridges, opts: &RuntimeOptions) -> String {
    let dropped: Vec<&str> = plan.dropped.iter().map(|(n, _)| n.as_str()).collect();
    let mut proxies = Vec::new();
    let mut names = Vec::new();
    for n in nodes.iter().filter(|n| !dropped.contains(&n.name.as_str())) {
        let p = match bridges.get(&n.name) {
            Some(port) => json!({ "name": n.name, "type": "socks5", "server": "127.0.0.1", "port": port, "udp": true }),
            None => to_clash(n),
        };
        proxies.push(p);
        names.push(n.name.clone());
    }

    let groups = if names.is_empty() {
        json!([{ "name": PROXY, "type": "select", "proxies": ["DIRECT"] }])
    } else {
        let mut sel = vec![AUTO.to_string(), FALLBACK.to_string()];
        sel.extend(names.iter().cloned());
        json!([
            { "name": PROXY, "type": "select", "proxies": sel },
            { "name": AUTO, "type": "url-test", "proxies": names, "url": TEST_URL, "interval": 300, "tolerance": 50, "lazy": true },
            { "name": FALLBACK, "type": "fallback", "proxies": names, "url": TEST_URL, "interval": 300, "lazy": true },
        ])
    };

    let known: HashSet<&str> = names.iter().map(String::as_str).chain([AUTO, FALLBACK]).collect();
    let mut rules = bypass_rules(opts);
    rules.extend(opts.rules().iter().map(|r| r.retargeted(|t| known.contains(t)).to_clash(PROXY)));

    let mut doc = base(opts);
    doc["sniffer"] = if opts.sniff { sniffer() } else { json!({ "enable": false }) };
    doc["dns"] = dns(opts);
    doc["tun"] = tun(opts);
    doc["proxies"] = Value::Array(proxies);
    doc["proxy-groups"] = groups;
    let mut doc: Mapping = serde_yaml_ng::from_value(serde_yaml_ng::to_value(doc).expect("json to yaml")).expect("an object");
    rules.extend(apply_smart(&mut doc, opts, PROXY));
    rules.extend(opts.tail_rules().iter().map(|r| r.to_clash(PROXY)));
    doc.insert(Yaml::String("rules".into()), serde_yaml_ng::to_value(rules).expect("strings"));
    serde_yaml_ng::to_string(&doc).expect("JSON values always serialise to YAML")
}

/// Keys we own in an imported Clash profile. Ports are removed so they can't clash
/// with ours; the controller is ours so the UI can talk to the core.
const STRIP: &[&str] = &[
    "port", "socks-port", "redir-port", "tproxy-port", "external-controller-tls", "external-controller-unix",
    "external-controller-pipe", "external-ui", "external-ui-url", "external-ui-name", "authentication",
];

/// Take an imported Clash config and apply app-controlled settings on top.
pub fn merge_clash(raw: &str, opts: &RuntimeOptions) -> Result<String, GenError> {
    let mut doc: Mapping = serde_yaml_ng::from_str(raw).map_err(|e| GenError::Yaml(e.to_string()))?;
    for k in STRIP {
        doc.remove(*k);
    }
    let set = |doc: &mut Mapping, k: &str, v: Value| {
        doc.insert(Yaml::String(k.into()), serde_yaml_ng::to_value(v).expect("json to yaml"));
    };
    if let Value::Object(b) = base(opts) {
        for (k, v) in b {
            // Keep the profile's own geodata choices, unless the user picked a source.
            let user_picked_source = opts.geodata != GeoSource::Default;
            if !user_picked_source && matches!(k.as_str(), "geodata-mode" | "geox-url" | "geo-auto-update" | "geo-update-interval") && doc.contains_key(k.as_str()) {
                continue;
            }
            set(&mut doc, &k, v);
        }
    }
    set(&mut doc, "tun", tun(opts));
    if opts.dns.override_profile || !doc.contains_key("dns") {
        set(&mut doc, "dns", dns(opts));
    }
    if !doc.contains_key("sniffer") && opts.sniff {
        set(&mut doc, "sniffer", sniffer());
    }

    let main = main_group(&doc);
    let known = names_in(&doc);
    let mut ours: Vec<String> = bypass_rules(opts);
    let mut learned = Vec::new();
    if opts.apply_rules_to_clash_profiles {
        ours.extend(opts.rules().iter().map(|r| r.retargeted(|t| known.contains(t)).to_clash(&main)));
        learned = apply_smart(&mut doc, opts, &main);
    }
    if !ours.is_empty() || !learned.is_empty() {
        let mut existing = match doc.remove("rules") {
            Some(Yaml::Sequence(s)) => s,
            _ => Vec::new(),
        };
        let mut merged: Vec<Yaml> = ours.into_iter().map(Yaml::String).collect();
        let learned = learned.into_iter().map(Yaml::String);
        if opts.apply_rules_to_clash_profiles && blocked_only(opts) {
            // "Only blocked sites through the proxy": everything after our rules goes
            // direct, so the profile's own catch-all must not run.
            merged.extend(learned);
            merged.extend(opts.tail_rules().iter().map(|r| Yaml::String(r.to_clash(&main))));
        } else {
            // Learned from traffic only the catch-all matched, learned sites go right before
            // it, so the profile's own rules keep deciding everything else. When smart
            // routing may also correct the profile's rules, they must come first.
            let catch_all = if opts.smart_profile_rules {
                0
            } else {
                existing.iter().position(|r| r.as_str().is_some_and(|r| r.trim_start().starts_with("MATCH,"))).unwrap_or(existing.len())
            };
            existing.splice(catch_all..catch_all, learned);
            merged.extend(existing);
        }
        doc.insert(Yaml::String("rules".into()), Yaml::Sequence(merged));
    }
    serde_yaml_ng::to_string(&doc).map_err(|e| GenError::Yaml(e.to_string()))
}

/// Keys a config run with system rights must not have: they serve files from disk or
/// open extra control channels.
const SERVICE_STRIP: &[&str] = &[
    "external-ui", "external-ui-url", "external-ui-name", "external-controller-tls", "external-controller-unix",
    "external-controller-pipe", "external-controller-cors", "external-doh-server",
];

/// Prepare a config for the privileged service. The controller must listen on loopback
/// and require a secret; anything that could expose the filesystem is removed. Provider
/// paths are confined to the home directory by Mihomo itself.
pub fn sanitize_for_service(raw: &str) -> Result<String, GenError> {
    let mut doc: Mapping = serde_yaml_ng::from_str(raw).map_err(|e| GenError::Yaml(e.to_string()))?;
    for k in SERVICE_STRIP {
        doc.remove(*k);
    }
    let secret = doc.get("secret").and_then(|s| s.as_str()).unwrap_or_default();
    if secret.len() < 16 {
        return Err(GenError::Yaml("the controller secret is missing or too short".into()));
    }
    let controller = doc.get("external-controller").and_then(|s| s.as_str()).unwrap_or_default();
    let loopback = controller.strip_prefix("127.0.0.1:").and_then(|p| p.parse::<u16>().ok()).is_some();
    if !loopback {
        return Err(GenError::Yaml("the controller must listen on 127.0.0.1".into()));
    }
    serde_yaml_ng::to_string(&doc).map_err(|e| GenError::Yaml(e.to_string()))
}

/// Names of every proxy and group defined in a Clash config.
fn names_in(doc: &Mapping) -> HashSet<String> {
    ["proxies", "proxy-groups"]
        .iter()
        .filter_map(|k| doc.get(*k).and_then(|v| v.as_sequence()))
        .flatten()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect()
}

/// [`main_group`] for a raw Clash config; `GLOBAL` if it can't be read.
pub fn main_group_of(raw: &str) -> String {
    serde_yaml_ng::from_str::<Mapping>(raw).map(|d| main_group(&d)).unwrap_or_else(|_| "GLOBAL".into())
}

/// The group that plays the role of our `Proxy`: one named so, else the first `select`.
pub fn main_group(doc: &Mapping) -> String {
    let groups = match doc.get("proxy-groups") {
        Some(Yaml::Sequence(g)) => g.clone(),
        _ => return "GLOBAL".into(),
    };
    let name_of = |g: &Yaml| g.get("name").and_then(|n| n.as_str()).map(String::from);
    let is_select = |g: &Yaml| g.get("type").and_then(|t| t.as_str()) == Some("select");
    groups
        .iter()
        .find(|g| name_of(g).as_deref() == Some(PROXY))
        .or_else(|| groups.iter().find(|g| is_select(g)))
        .and_then(name_of)
        .unwrap_or_else(|| "GLOBAL".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::{CorePreference, plan};
    use crate::links::parse_link;

    fn opts() -> RuntimeOptions {
        RuntimeOptions { secret: "s3cret".into(), ..Default::default() }
    }

    #[test]
    fn nodes_profile_has_groups_rules_and_bridges() {
        let nodes = vec![
            parse_link("vless://id@a.example:443?security=tls&type=xhttp&path=/x#XH").unwrap(),
            parse_link("trojan://pw@b.example:443#TR").unwrap(),
        ];
        let p = plan(&nodes, CorePreference::Auto);
        let bridges = Bridges::from([("XH".to_string(), 17001)]);
        let y = from_nodes(&nodes, &p, &bridges, &opts());
        let doc: Value = serde_yaml_ng::from_str(&y).unwrap();
        assert_eq!(doc["mixed-port"], 7890);
        assert_eq!(doc["external-controller"], "127.0.0.1:9097");
        let proxies = doc["proxies"].as_array().unwrap();
        assert_eq!(proxies[0]["type"], "socks5");
        assert_eq!(proxies[0]["port"], 17001);
        assert_eq!(proxies[1]["type"], "trojan");
        assert_eq!(doc["proxy-groups"][0]["proxies"][0], AUTO);
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        assert!(rules.contains(&"GEOIP,RU,DIRECT"));
        assert!(rules.contains(&"GEOSITE,category-ads-all,REJECT"));
        assert_eq!(*rules.last().unwrap(), "MATCH,Proxy");
        assert!(doc["dns"]["nameserver-policy"]["geosite:category-ru"].is_array());
    }

    #[test]
    fn bridge_traffic_bypasses_tun_and_rules() {
        let nodes = vec![parse_link("trojan://pw@b.example:443#TR").unwrap()];
        let p = plan(&nodes, CorePreference::Auto);
        let mut o = opts();
        o.bypass_processes = vec!["xray.exe".into()];
        o.tun_exclude = vec!["203.0.113.9/32".into()];
        let doc: Value = serde_yaml_ng::from_str(&from_nodes(&nodes, &p, &Bridges::new(), &o)).unwrap();
        assert_eq!(doc["rules"][0], "PROCESS-NAME,xray.exe,DIRECT");
        assert_eq!(doc["tun"]["route-exclude-address"][0], "203.0.113.9/32");
        // Per-run values are never stored with the settings.
        let saved: RuntimeOptions = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert!(saved.bypass_processes.is_empty() && saved.tun_exclude.is_empty());
    }

    #[test]
    fn blocked_only_preset_overrides_a_clash_catch_all() {
        let raw = "proxies:\n  - {name: A, type: trojan, server: a, port: 443, password: x}\nproxy-groups:\n  - {name: Sel, type: select, proxies: [A]}\nrules:\n  - DOMAIN,x.example,Sel\n  - MATCH,Sel\n";
        let mut o = opts();
        o.presets = vec![Preset::ProxyBlockedRu];
        let doc: Value = serde_yaml_ng::from_str(&merge_clash(raw, &o).unwrap()).unwrap();
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        assert_eq!(rules[0], "GEOSITE,ru-blocked,Sel");
        assert_eq!(*rules.last().unwrap(), "MATCH,DIRECT");
        assert!(!rules.contains(&"MATCH,Sel"));
    }

    #[test]
    fn smart_routing_lists_come_before_the_catch_all() {
        let nodes = vec![parse_link("trojan://pw@b.example:443#TR").unwrap()];
        let p = plan(&nodes, CorePreference::Auto);
        let mut o = opts();
        o.presets = vec![Preset::ProxyBlockedRu];
        o.smart = Some(SmartRuntime { lists_url: "http://127.0.0.1:5000/t/".into(), direct_port: 5001, proxy_port: 5002, scope: Default::default() });
        let doc: Value = serde_yaml_ng::from_str(&from_nodes(&nodes, &p, &Bridges::new(), &o)).unwrap();
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        let at = |r: &str| rules.iter().position(|x| x.starts_with(r)).unwrap();
        assert!(at("GEOSITE,ru-blocked") < at("RULE-SET,kova-learned-direct,DIRECT"));
        assert!(at("RULE-SET,kova-learned-proxy,Proxy") < at("GEOSITE,private"));
        assert!(at("RULE-SET,kova-learned-proxy-ip,Proxy,no-resolve") < at("GEOSITE,private"));
        assert_eq!(doc["rule-providers"]["kova-learned-proxy-ip"]["behavior"], "ipcidr");
        assert_eq!(doc["rule-providers"]["kova-learned-proxy"]["behavior"], "domain");
        assert_eq!(*rules.last().unwrap(), "MATCH,DIRECT");
        assert_eq!(doc["rule-providers"]["kova-learned-proxy"]["url"], "http://127.0.0.1:5000/t/kova-learned-proxy.txt");
        assert_eq!(doc["listeners"][0]["proxy"], "DIRECT");
        assert_eq!(doc["listeners"][1]["proxy"], "Proxy");
        assert_eq!(doc["listeners"][1]["listen"], "127.0.0.1");

        o.smart = None;
        let plain: Value = serde_yaml_ng::from_str(&from_nodes(&nodes, &p, &Bridges::new(), &o)).unwrap();
        assert!(plain["rule-providers"].is_null() && plain["listeners"].is_null());
    }

    #[test]
    fn smart_routing_keeps_a_clash_profiles_own_listeners() {
        let raw = "proxies:
  - {name: A, type: trojan, server: a, port: 443, password: x}
proxy-groups:
  - {name: Sel, type: select, proxies: [A]}
listeners:
  - {name: mine, type: socks, port: 7000}
rules:
  - DOMAIN,mine.example,DIRECT
  - MATCH,Sel
";
        let mut o = opts();
        o.smart = Some(SmartRuntime { lists_url: "http://127.0.0.1:5000/t/".into(), direct_port: 5001, proxy_port: 5002, scope: Default::default() });
        let doc: Value = serde_yaml_ng::from_str(&merge_clash(raw, &o).unwrap()).unwrap();
        let names: Vec<&str> = doc["listeners"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["mine", "kova-probe-direct", "kova-probe-proxy"]);
        assert_eq!(doc["listeners"][2]["proxy"], "Sel");
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        // The profile's own rules still come first; learned sites only precede its catch-all.
        let at = |r: &str| rules.iter().position(|x| *x == r).unwrap();
        assert!(at("RULE-SET,kova-learned-proxy,Sel") < at("MATCH,Sel"));
        assert!(at("DOMAIN,mine.example,DIRECT") < at("RULE-SET,kova-learned-direct,DIRECT"));
        assert_eq!(*rules.last().unwrap(), "MATCH,Sel");

        // Allowed to correct the profile's rules: learned sites come before them.
        o.smart_profile_rules = true;
        let doc: Value = serde_yaml_ng::from_str(&merge_clash(raw, &o).unwrap()).unwrap();
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        let at = |r: &str| rules.iter().position(|x| *x == r).unwrap();
        assert!(at("RULE-SET,kova-learned-proxy,Sel") < at("DOMAIN,mine.example,DIRECT"));
    }

    #[test]
    fn own_rules_are_named_as_mihomo_logs_them() {
        let mut o = opts();
        o.presets = vec![Preset::DirectRu];
        o.custom_rules = vec![Rule { kind: RuleKind::DomainSuffix, value: "example.org".into(), target: PROXY.into() }];
        o.bypass_processes = vec!["xray.exe".into()];
        let s = smart_scope(&o, false);
        for k in ["GeoSite(category-ru)", "GeoIP(RU)"] {
            assert!(s.preset_rules.contains(k), "{k} missing from {:?}", s.preset_rules);
        }
        assert!(s.user_rules.contains("DomainSuffix(example.org)"));
        for k in ["IPCIDR(10.0.0.0/8)", "ProcessName(xray.exe)", "RuleSet(kova-learned-proxy)"] {
            assert!(s.internal_rules.contains(k), "{k} missing from {:?}", s.internal_rules);
        }
        assert!(!s.user_rules.contains("GeoSite(category-games)"));
    }

    #[test]
    fn android_uses_the_interface_it_is_given() {
        let nodes = vec![parse_link("trojan://pw@b.example:443#TR").unwrap()];
        let p = plan(&nodes, CorePreference::Auto);
        let mut o = opts();
        o.tun.enabled = true;
        o.tun_fd = Some(42);
        let doc: Value = serde_yaml_ng::from_str(&from_nodes(&nodes, &p, &Bridges::new(), &o)).unwrap();
        assert_eq!(doc["tun"]["file-descriptor"], 42);
        assert_eq!(doc["tun"]["auto-route"], false);
        assert_eq!(doc["find-process-mode"], "off");
        let saved: RuntimeOptions = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(saved.tun_fd, None, "a descriptor is never stored");
    }

    #[test]
    fn service_configs_are_locked_down() {
        let good = "secret: 0123456789abcdef0123\nexternal-controller: 127.0.0.1:61000\nexternal-ui: C:\\\\\nexternal-controller-pipe: \\\\.\\pipe\\x\n";
        let out: Value = serde_yaml_ng::from_str(&sanitize_for_service(good).unwrap()).unwrap();
        assert!(out.get("external-ui").is_none());
        assert!(out.get("external-controller-pipe").is_none());
        assert_eq!(out["external-controller"], "127.0.0.1:61000");

        assert!(sanitize_for_service("external-controller: 127.0.0.1:1\n").is_err(), "no secret");
        assert!(sanitize_for_service("secret: 0123456789abcdef0123\nexternal-controller: 0.0.0.0:9090\n").is_err(), "not loopback");
        assert!(sanitize_for_service("secret: 0123456789abcdef0123\n").is_err(), "no controller");
    }

    #[test]
    fn rules_for_missing_groups_fall_back_to_the_main_group() {
        let nodes = vec![parse_link("trojan://pw@b.example:443#TR").unwrap()];
        let p = plan(&nodes, CorePreference::Auto);
        let mut o = opts();
        o.presets.clear();
        o.custom_rules = vec![
            Rule { kind: RuleKind::DomainSuffix, value: "netflix.com".into(), target: "Streaming".into() },
            Rule { kind: RuleKind::DomainSuffix, value: "a.example".into(), target: "TR".into() },
            Rule { kind: RuleKind::DomainSuffix, value: "b.example".into(), target: "REJECT".into() },
        ];
        let doc: Value = serde_yaml_ng::from_str(&from_nodes(&nodes, &p, &Bridges::new(), &o)).unwrap();
        let rules: Vec<&str> = doc["rules"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        assert_eq!(&rules[..3], ["DOMAIN-SUFFIX,netflix.com,Proxy", "DOMAIN-SUFFIX,a.example,TR", "DOMAIN-SUFFIX,b.example,REJECT"]);

        let raw = "proxies:
  - {name: A, type: trojan, server: a, port: 443, password: x}
proxy-groups:
  - {name: Media, type: select, proxies: [A]}
rules:
  - MATCH,Media
";
        let merged: Value = serde_yaml_ng::from_str(&merge_clash(raw, &o).unwrap()).unwrap();
        assert_eq!(merged["rules"][0], "DOMAIN-SUFFIX,netflix.com,Media");
        assert_eq!(merged["rules"][1], "DOMAIN-SUFFIX,a.example,Media");
        assert_eq!(main_group_of(raw), "Media");
    }

    #[test]
    fn clash_merge_overrides_ports_and_prepends_rules() {
        let raw = "port: 7891\nsocks-port: 7892\nmixed-port: 1\nexternal-controller: 0.0.0.0:9090\nsecret: theirs\nproxies:\n  - {name: A, type: trojan, server: a, port: 443, password: x}\nproxy-groups:\n  - {name: Select, type: select, proxies: [A]}\nrules:\n  - DOMAIN-SUFFIX,example.com,Select\n  - MATCH,Select\n";
        let mut o = opts();
        o.custom_rules.push(Rule { kind: RuleKind::DomainSuffix, value: "example.org".into(), target: PROXY.into() });
        let y = merge_clash(raw, &o).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&y).unwrap();
        assert!(doc.get("port").is_none());
        assert!(doc.get("socks-port").is_none());
        assert_eq!(doc["mixed-port"], 7890);
        assert_eq!(doc["secret"], "s3cret");
        assert_eq!(doc["external-controller"], "127.0.0.1:9097");
        let rules = doc["rules"].as_array().unwrap();
        assert_eq!(rules[0], "DOMAIN-SUFFIX,example.org,Select");
        assert_eq!(rules.last().unwrap(), "MATCH,Select");
        assert_eq!(doc["tun"]["enable"], false);
    }
}
