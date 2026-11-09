//! Sub-Store style Clash profiles (issue #10): whatever Kova does to an imported profile
//! must leave every proxy group with the servers it was meant to list. Mihomo takes a
//! group's servers from `proxies:`, from `use:` (proxy providers) and from `include-all`,
//! so those keys, the providers they name and the group filters must come out unchanged.

use kova_core::gen_mihomo::merge_clash;
use kova_core::options::{Rule, RuleKind};
use kova_core::RuntimeOptions;
use serde_yaml_ng::{Mapping, Value};

/// Sub-Store "file" template: groups fed by proxy providers (the usual way to keep Sub-Store
/// in charge of the server list), the provider URL asks Sub-Store for the internal format.
const PROVIDERS: &str = r#"
mixed-port: 7890
mode: rule
proxy-providers:
  SubStore:
    type: http
    url: "https://substore.example.com/download/collection/main?target=ClashMeta&produceType=internal"
    path: ./providers/substore.yaml
    interval: 3600
    health-check: {enable: true, url: "https://www.gstatic.com/generate_204", interval: 300}
    filter: "^(?!.*(Expire|Traffic))"
  Second:
    type: http
    url: "https://substore.example.com/download/second?target=ClashMeta&produceType=internal"
    path: ./providers/second.yaml
    interval: 3600
proxy-groups:
  - {name: Proxy, type: select, proxies: [Auto, HK, DIRECT], use: [SubStore, Second]}
  - {name: Auto, type: url-test, use: [SubStore], url: "https://www.gstatic.com/generate_204", interval: 300}
  - {name: HK, type: select, use: [SubStore], filter: "(?i)hk|hong"}
  - {name: Everything, type: select, include-all: true}
  - {name: Nodes, type: select, include-all-proxies: true, filter: "(?i)jp|japan"}
rules:
  - DOMAIN-SUFFIX,example.com,HK
  - MATCH,Proxy
"#;

/// Sub-Store plain Clash output: every server inline, groups listing them by name.
const INLINE: &str = r#"
proxies:
  - {name: "🇭🇰 HK 01", type: trojan, server: hk1.example.com, port: 443, password: x}
  - {name: "🇯🇵 JP 01", type: trojan, server: jp1.example.com, port: 443, password: x}
  - {name: "🇺🇸 US 01", type: trojan, server: us1.example.com, port: 443, password: x}
proxy-groups:
  - {name: Proxy, type: select, proxies: [Auto, "🇭🇰 HK 01", "🇯🇵 JP 01", "🇺🇸 US 01", DIRECT]}
  - {name: Auto, type: url-test, proxies: ["🇭🇰 HK 01", "🇯🇵 JP 01", "🇺🇸 US 01"], url: "https://www.gstatic.com/generate_204", interval: 300}
rules:
  - MATCH,Proxy
"#;

fn parse(s: &str) -> Mapping {
    serde_yaml_ng::from_str(s).unwrap()
}

fn groups(doc: &Mapping) -> Vec<Mapping> {
    doc["proxy-groups"].as_sequence().unwrap().iter().map(|g| g.as_mapping().unwrap().clone()).collect()
}

/// Every key that decides which servers a group lists.
const MEMBERSHIP: &[&str] = &["proxies", "use", "include-all", "include-all-proxies", "include-all-providers", "filter", "exclude-filter", "exclude-type"];

/// The profile's own groups (by name) keep their membership keys, whatever options are on.
fn assert_groups_intact(input: &str, opts: &RuntimeOptions, label: &str) {
    let before = parse(input);
    let out = merge_clash(input, opts).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    let after = parse(&out);
    let after_groups = groups(&after);
    for g in groups(&before) {
        let name = g["name"].as_str().unwrap();
        let now = after_groups.iter().find(|a| a["name"].as_str() == Some(name)).unwrap_or_else(|| panic!("{label}: group {name} is gone"));
        for key in MEMBERSHIP {
            assert_eq!(g.get(*key), now.get(*key), "{label}: group {name}: `{key}` changed\n{out}");
        }
    }
    // The providers the groups name must still be there, with their URL.
    if let Some(Value::Mapping(p)) = before.get("proxy-providers") {
        for (name, body) in p {
            let now = &after["proxy-providers"][name.as_str().unwrap()];
            assert_eq!(body["url"], now["url"], "{label}: provider {name:?} lost its url");
            assert_eq!(body["type"], now["type"], "{label}: provider {name:?} changed type");
        }
    }
}

fn option_sets() -> Vec<(&'static str, RuntimeOptions)> {
    let rule = Rule { kind: RuleKind::Geosite, value: "netflix".into(), target: "country:JP".into() };
    vec![
        ("defaults", RuntimeOptions::default()),
        ("kova rules + smart", RuntimeOptions { apply_rules_to_clash_profiles: true, custom_rules: vec![rule], smart_routing: true, smart_profile_rules: true, ..Default::default() }),
        ("anti-dpi", RuntimeOptions { anti_dpi: true, anti_dpi_port: Some(17100), local_password: "pw".into(), ..Default::default() }),
        ("hide RU + http", RuntimeOptions { hidden_countries: vec!["RU".into()], hidden_protocols: vec!["Shadowsocks".into()], ..Default::default() }),
        ("without geodata", RuntimeOptions { without_geodata: true, ..Default::default() }),
    ]
}

#[test]
fn provider_groups_keep_their_servers() {
    for (label, o) in option_sets() {
        assert_groups_intact(PROVIDERS, &o, &format!("providers / {label}"));
    }
}

#[test]
fn inline_groups_keep_their_servers() {
    for (label, o) in option_sets() {
        assert_groups_intact(INLINE, &o, &format!("inline / {label}"));
    }
}

#[test]
fn providers_with_nothing_inline_still_work_with_kova_rules() {
    // A profile that has no `proxies:` at all: nothing Kova adds may take the place of the
    // provider's servers or empty a group.
    let (_, o) = option_sets().remove(1);
    let out = parse(&merge_clash(PROVIDERS, &o).unwrap());
    let main = out["proxy-groups"].as_sequence().unwrap().iter().find(|g| g["name"].as_str() == Some("Proxy")).unwrap();
    assert_eq!(main["use"].as_sequence().unwrap().len(), 2, "{main:?}");
    let rules: Vec<&str> = out["rules"].as_sequence().unwrap().iter().filter_map(|r| r.as_str()).collect();
    assert!(rules.last().is_some_and(|r| r.starts_with("MATCH,")), "{rules:?}");
}

#[test]
fn print_generated_config() {
    // Run with `cargo test -p kova-core --test substore_profiles print -- --nocapture`.
    let (_, o) = option_sets().remove(1);
    println!("{}", merge_clash(PROVIDERS, &o).unwrap());
}
