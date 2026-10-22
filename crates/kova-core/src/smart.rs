//! Smart routing: sites Kova learned to route differently because the usual path fails.
//! A site blocked on the direct path goes through the proxy; a site that refuses the
//! proxy goes direct. This module is the pure part: reading what Mihomo reports and
//! keeping the learned list. The engine watches, probes and applies.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// Mihomo rule providers holding the learned sites: names and bare IP addresses (apps
/// that connect without a name, mostly seen in TUN mode), one list per route.
pub const PROVIDER_PROXY: &str = "kova-learned-proxy";
pub const PROVIDER_DIRECT: &str = "kova-learned-direct";
pub const PROVIDER_PROXY_IP: &str = "kova-learned-proxy-ip";
pub const PROVIDER_DIRECT_IP: &str = "kova-learned-direct-ip";
pub const PROVIDERS: [&str; 4] = [PROVIDER_PROXY, PROVIDER_DIRECT, PROVIDER_PROXY_IP, PROVIDER_DIRECT_IP];

/// A learned site is forgotten after this long unless it is learned again.
pub const MAX_AGE_SECS: i64 = 30 * 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    Proxy,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Learned {
    /// The rule covers this domain and its subdomains, or this one IP address.
    pub domain: String,
    pub route: Route,
    /// How the usual path failed: a site-check outcome such as "reset" or "timeout".
    pub reason: String,
    /// Unix seconds it was last learned.
    pub learned_at: i64,
}

/// The catch-all rule's key (see [`rule_key`]).
pub const CATCH_ALL: &str = "Match";

/// How Mihomo names a rule in its logs: `GeoSite(category-games)`, or just `Match`
/// for a rule without a value. Kova compares rules by this key to tell its own rules
/// from a subscription's.
pub fn rule_key(kind: &str, payload: &str) -> String {
    if payload.is_empty() { kind.to_string() } else { format!("{kind}({payload})") }
}

/// A connection Mihomo could not open, from its "dial ... error" warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialFailure {
    /// The connection was to go direct (otherwise through a proxy or group).
    pub direct: bool,
    /// The rule that chose its route, as a [`rule_key`].
    pub rule: String,
    pub host: String,
    pub port: u16,
}

/// Read a Mihomo warning such as
/// `[TCP] dial DIRECT (match Match/) 127.0.0.1:50812(chrome.exe) --> example.org:443 error: i/o timeout`.
pub fn parse_dial_failure(line: &str) -> Option<DialFailure> {
    let rest = line.strip_prefix("[TCP] dial ")?;
    let (proxy, rest) = rest.split_once(" (match ")?;
    let (rule, rest) = rest.split_once(") ")?;
    let (_source, rest) = rest.split_once(" --> ")?;
    let (dest, _error) = rest.split_once(" error: ")?;
    let (host, port) = dest.rsplit_once(':')?;
    if proxy == "REJECT" || proxy.starts_with("REJECT-") {
        return None;
    }
    let (kind, payload) = rule.split_once('/').unwrap_or((rule, ""));
    Some(DialFailure { direct: proxy == "DIRECT", rule: rule_key(kind, payload), host: host.to_string(), port: port.parse().ok()? })
}

/// A connection Mihomo opened, from its info line
/// `[TCP] 127.0.0.1:62890(curl.exe) --> modrinth.com:443 match GeoSite(category-games) using DIRECT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// It goes direct (otherwise through a proxy or group).
    pub direct: bool,
    /// The rule that chose its route, as a [`rule_key`].
    pub rule: String,
    pub host: String,
    pub port: u16,
}

pub fn parse_opened(line: &str) -> Option<Opened> {
    let rest = line.strip_prefix("[TCP] ")?;
    let (_source, rest) = rest.split_once(" --> ")?;
    let (dest, rest) = rest.split_once(" match ")?;
    let (rule, chain) = rest.rsplit_once(" using ")?;
    let (host, port) = dest.rsplit_once(':')?;
    if chain == "REJECT" || chain.starts_with("REJECT-") {
        return None;
    }
    Some(Opened { direct: chain == "DIRECT", rule: rule.to_string(), host: host.to_string(), port: port.parse().ok()? })
}

/// Where the rule that chose a route comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// One of the user's own rules.
    User,
    /// A routing preset.
    Preset,
    /// An imported Clash profile's own rules.
    Profile,
    /// No rule known: Site check found the site opens only directly.
    SiteCheck,
}

/// What smart routing does about a route that fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Learn the site: no rule chose its route on purpose (or the user lets Kova
    /// correct the subscription's rules).
    Learn,
    /// Only suggest an exception: a rule chose the route on purpose, and that rule
    /// is the user's to change.
    Suggest(Origin),
}

/// Which rule is whose, to decide what smart routing does about a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub user_rules: std::collections::HashSet<String>,
    pub preset_rules: std::collections::HashSet<String>,
    /// Kova's internal rules (its own processes, the learned lists, the local
    /// network), which are never questioned.
    pub internal_rules: std::collections::HashSet<String>,
    /// Learn over a subscription's own rules instead of only suggesting.
    pub profile_rules: bool,
    /// Act on sites that fail through the proxy (checking them directly). Off, Kova
    /// never goes direct on its own.
    pub proxy_failures: bool,
}

impl Scope {
    pub fn action(&self, rule: &str) -> Option<Action> {
        if rule == CATCH_ALL {
            Some(Action::Learn)
        } else if self.user_rules.contains(rule) {
            Some(Action::Suggest(Origin::User))
        } else if self.preset_rules.contains(rule) {
            Some(Action::Suggest(Origin::Preset))
        } else if self.internal_rules.contains(rule) {
            None
        } else if self.profile_rules {
            // Any other rule can only come from the imported profile.
            Some(Action::Learn)
        } else {
            Some(Action::Suggest(Origin::Profile))
        }
    }
}

/// A site that fails on the route a rule chose on purpose, and works the other way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub domain: String,
    /// The way that works.
    pub route: Route,
    pub reason: String,
    /// The rule that chose the failing route, as a [`rule_key`].
    pub rule: String,
    pub origin: Origin,
    pub at: i64,
}

impl Learned {
    pub fn is_ip(&self) -> bool {
        self.domain.parse::<IpAddr>().is_ok()
    }
}

/// An IP address written as Mihomo prints it, with or without IPv6 brackets.
pub fn parse_ip(host: &str) -> Option<IpAddr> {
    host.trim().trim_start_matches('[').trim_end_matches(']').parse().ok()
}

/// Whether an address is a public one worth a rule. Private and special ranges are
/// left out, and so is Mihomo's fake-IP range: those addresses stand for names.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            let special = v.is_private() || v.is_loopback() || v.is_link_local() || v.is_broadcast() || v.is_documentation() || v.is_unspecified() || v.is_multicast();
            let reserved = a == 0 || a >= 240 || (a == 100 && (64..128).contains(&b)) || (a == 198 && (18..20).contains(&b)) || (a == 192 && b == 0 && c == 0);
            !special && !reserved
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && s[1] == 0x0db8)
        }
    }
}

/// Address ranges of the big CDNs (Cloudflare, Fastly, Amazon CloudFront), from their
/// published lists. One such address serves thousands of unrelated sites, so a rule for
/// it would move all of them; these are never learned.
const SHARED_CDN: &[&str] = &[
    "173.245.48.0/20", "103.21.244.0/22", "103.22.200.0/22", "103.31.4.0/22", "141.101.64.0/18", "108.162.192.0/18",
    "190.93.240.0/20", "188.114.96.0/20", "197.234.240.0/22", "198.41.128.0/17", "162.158.0.0/15", "104.16.0.0/13",
    "104.24.0.0/14", "172.64.0.0/13", "131.0.72.0/22", "2400:cb00::/32", "2606:4700::/32", "2803:f800::/32",
    "2405:b500::/32", "2405:8100::/32", "2a06:98c0::/29", "2c0f:f248::/32",
    "151.101.0.0/16", "199.232.0.0/16", "146.75.0.0/16", "2a04:4e40::/32",
    "13.32.0.0/15", "13.224.0.0/14", "18.64.0.0/14", "52.84.0.0/15", "54.230.0.0/16", "54.239.128.0/18", "99.84.0.0/16",
    "143.204.0.0/16", "205.251.192.0/19", "2600:9000::/28",
];

pub fn is_shared_cdn(ip: IpAddr) -> bool {
    SHARED_CDN.iter().any(|net| {
        let (base, len) = net.split_once('/').expect("a CIDR");
        let len: u32 = len.parse().expect("a prefix length");
        match (ip, base.parse::<IpAddr>().expect("an address")) {
            (IpAddr::V4(a), IpAddr::V4(b)) => len == 0 || (u32::from(a) ^ u32::from(b)) >> (32 - len) == 0,
            (IpAddr::V6(a), IpAddr::V6(b)) => len == 0 || (u128::from(a) ^ u128::from(b)) >> (128 - len) == 0,
            _ => false,
        }
    })
}

/// What a rule should cover for a connection's destination: the registrable domain
/// for a name (see [`rule_domain`]) or the address itself for a public IP.
pub fn rule_target(host: &str) -> Option<String> {
    match parse_ip(host) {
        Some(ip) => (is_public_ip(ip) && !is_shared_cdn(ip)).then(|| ip.to_string()),
        None => rule_domain(host),
    }
}

/// The domain a rule should cover for `host`: its registrable domain, so that
/// `rr3---sn-abc.googlevideo.com` gives `googlevideo.com` while `someone.github.io`
/// stays itself. `None` for IP addresses and names that are not public.
pub fn rule_domain(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if parse_ip(&host).is_some() || !host.contains('.') || !is_dns_name(&host) {
        return None;
    }
    let d = psl::domain(host.as_bytes())?;
    if !d.suffix().is_known() {
        return None;
    }
    std::str::from_utf8(d.as_bytes()).ok().map(String::from)
}

/// A plain DNS name: dot-separated labels of letters, digits, `-` and `_` (punycode for
/// other scripts). Names come from what apps asked for, and one with a comma or a line
/// break would end up as a broken rule or as extra lines in a learned list.
fn is_dns_name(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
}

/// Whether `host` is `domain` or one of its subdomains, or is that IP address.
pub fn covers(domain: &str, host: &str) -> bool {
    if let (Some(a), Some(b)) = (parse_ip(domain), parse_ip(host)) {
        return a == b;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
}

/// Add a site, or refresh it if it is already known (possibly with the other route).
pub fn upsert(list: &mut Vec<Learned>, item: Learned) {
    list.retain(|l| l.domain != item.domain);
    list.push(item);
}

/// Drop sites learned longer ago than [`MAX_AGE_SECS`]. Returns whether any went.
pub fn prune(list: &mut Vec<Learned>, now: i64) -> bool {
    let before = list.len();
    list.retain(|l| now - l.learned_at < MAX_AGE_SECS);
    list.len() != before
}

/// The contents of one rule provider (text format): domains for the name lists,
/// CIDRs for the address lists. Each list always has a line, because Mihomo rejects an
/// empty one; the placeholder matches nothing real.
pub fn provider_text(list: &[Learned], provider: &str) -> Option<String> {
    let (route, ips) = match provider {
        PROVIDER_PROXY => (Route::Proxy, false),
        PROVIDER_DIRECT => (Route::Direct, false),
        PROVIDER_PROXY_IP => (Route::Proxy, true),
        PROVIDER_DIRECT_IP => (Route::Direct, true),
        _ => return None,
    };
    let mut out = String::from(if ips { "255.255.255.255/32\n" } else { "+.kova-learned.invalid\n" });
    for l in list.iter().filter(|l| l.route == route && l.is_ip() == ips) {
        match parse_ip(&l.domain) {
            Some(IpAddr::V4(ip)) => out.push_str(&format!("{ip}/32\n")),
            Some(IpAddr::V6(ip)) => out.push_str(&format!("{ip}/128\n")),
            None => out.push_str(&format!("+.{}\n", l.domain)),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_direct_and_proxy_dial_failures() {
        let d = parse_dial_failure("[TCP] dial DIRECT (match Match/) 127.0.0.1:50812(chrome.exe) --> rutracker.org:443 error: dial tcp 1.2.3.4:443: i/o timeout").unwrap();
        assert_eq!(d, DialFailure { direct: true, rule: CATCH_ALL.into(), host: "rutracker.org".into(), port: 443 });

        let p = parse_dial_failure("[TCP] dial 🚀 Proxy (match GeoSite/ru-blocked) 198.18.0.1:6000(Code Helper.exe) --> x.com:443 error: EOF").unwrap();
        assert!(!p.direct);
        assert_eq!(p.rule, "GeoSite(ru-blocked)");
        assert_eq!(p.host, "x.com");

        assert!(parse_dial_failure("[TCP] dial REJECT (match GeoSite/category-ads-all) a --> ads.example:443 error: x").is_none());
        assert!(parse_dial_failure("[UDP] dial DIRECT (match Match/) a --> b.com:443 error: x").is_none());
        assert!(parse_dial_failure("[TCP] 127.0.0.1:1 --> b.com:443 match Match using DIRECT").is_none());
    }

    #[test]
    fn reads_opened_connections_and_their_rule() {
        let o = parse_opened("[TCP] 127.0.0.1:62890(curl.exe) --> modrinth.com:443 match GeoSite(category-games) using DIRECT").unwrap();
        assert_eq!(o, Opened { direct: true, rule: "GeoSite(category-games)".into(), host: "modrinth.com".into(), port: 443 });
        let p = parse_opened("[TCP] 198.18.0.1:5000(Code Helper.exe) --> x.com:443 match Match using Remnawave[🇵🇱 hysteria-pl]").unwrap();
        assert!(!p.direct && p.rule == CATCH_ALL);
        assert!(parse_opened("[TCP] 127.0.0.1:1 --> ads.example:443 match GeoSite(category-ads-all) using REJECT").is_none());
        assert!(parse_opened("[TCP] dial DIRECT (match Match/) a --> b.com:443 error: x").is_none());
    }

    #[test]
    fn shared_cdn_addresses_are_never_learned() {
        for cdn in ["104.16.132.229", "151.101.1.69", "13.33.0.1", "2606:4700::6810:84e5"] {
            assert_eq!(rule_target(cdn), None, "{cdn}");
        }
        assert_eq!(rule_target("149.154.167.51").as_deref(), Some("149.154.167.51"));
    }

    #[test]
    fn only_plain_names_become_rules() {
        assert_eq!(rule_domain("cdn.Example.COM.").as_deref(), Some("example.com"));
        assert_eq!(rule_domain("xn--80ak6aa92e.com").as_deref(), Some("xn--80ak6aa92e.com"));
        for bad in ["x
+.com", "a,DIRECT.com", "a b.com", "a..com", ".com", "", &format!("{}.com", "a".repeat(64))] {
            assert_eq!(rule_domain(bad), None, "{bad:?}");
            assert_eq!(rule_target(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn learns_only_where_no_one_chose_and_suggests_elsewhere() {
        let set = |k: &[&str]| k.iter().map(|s| s.to_string()).collect();
        let mut scope = Scope {
            proxy_failures: true,
            user_rules: set(&["GeoSite(category-games)"]),
            preset_rules: set(&["GeoSite(category-ru)", "GeoSite(category-games)"]),
            internal_rules: set(&["GeoSite(private)", "RuleSet(kova-learned-proxy)"]),
            profile_rules: false,
        };
        assert_eq!(scope.action("Match"), Some(Action::Learn));
        assert_eq!(scope.action("GeoSite(category-games)"), Some(Action::Suggest(Origin::User)), "the user's rule wins over a preset's");
        assert_eq!(scope.action("GeoSite(category-ru)"), Some(Action::Suggest(Origin::Preset)));
        assert_eq!(scope.action("GeoSite(private)"), None);
        assert_eq!(scope.action("DomainSuffix(steamcontent.com)"), Some(Action::Suggest(Origin::Profile)));
        scope.profile_rules = true;
        assert_eq!(scope.action("DomainSuffix(steamcontent.com)"), Some(Action::Learn));
        assert_eq!(scope.action("GeoSite(category-games)"), Some(Action::Suggest(Origin::User)), "the user's rules are never overridden");
    }

    #[test]
    fn rules_cover_the_registrable_domain() {
        assert_eq!(rule_domain("rr3---sn-abc.googlevideo.com").as_deref(), Some("googlevideo.com"));
        assert_eq!(rule_domain("WWW.BBC.CO.UK.").as_deref(), Some("bbc.co.uk"));
        assert_eq!(rule_domain("someone.github.io").as_deref(), Some("someone.github.io"));
        assert_eq!(rule_domain("rutracker.org").as_deref(), Some("rutracker.org"));
        for not_public in ["1.2.3.4", "[2001:db8::1]", "2001:db8::1", "localhost", "printer.local", "nas.lan", "github.io"] {
            assert_eq!(rule_domain(not_public), None, "{not_public}");
        }
    }

    #[test]
    fn bare_addresses_get_rules_only_when_public() {
        assert_eq!(rule_target("149.154.167.51").as_deref(), Some("149.154.167.51"));
        assert_eq!(rule_target("[2001:67c:4e8:f004::9]").as_deref(), Some("2001:67c:4e8:f004::9"));
        assert_eq!(rule_target("cdn.example.org").as_deref(), Some("example.org"));
        // Private, special, and Mihomo's fake-IP range, whose addresses stand for names.
        for not_public in ["10.1.2.3", "192.168.1.1", "100.64.0.1", "198.18.0.7", "127.0.0.1", "192.0.2.1", "224.0.0.1", "fd00::1", "2001:db8::1", "::1"] {
            assert_eq!(rule_target(not_public), None, "{not_public}");
        }
        assert!(covers("149.154.167.51", "149.154.167.51") && !covers("149.154.167.51", "149.154.167.5"));
    }

    #[test]
    fn covers_subdomains_only() {
        assert!(covers("example.org", "example.org"));
        assert!(covers("example.org", "cdn.example.org."));
        assert!(!covers("example.org", "badexample.org"));
    }

    #[test]
    fn keeps_one_entry_per_domain_and_expires_old_ones() {
        let mut list = Vec::new();
        upsert(&mut list, Learned { domain: "a.com".into(), route: Route::Proxy, reason: "reset".into(), learned_at: 0 });
        upsert(&mut list, Learned { domain: "a.com".into(), route: Route::Direct, reason: "refused".into(), learned_at: 10 });
        upsert(&mut list, Learned { domain: "b.com".into(), route: Route::Proxy, reason: "timeout".into(), learned_at: 20 });
        assert_eq!(list.len(), 2);
        upsert(&mut list, Learned { domain: "149.154.167.51".into(), route: Route::Proxy, reason: "connect-failed".into(), learned_at: 30 });
        assert_eq!(provider_text(&list, PROVIDER_PROXY).unwrap(), "+.kova-learned.invalid\n+.b.com\n");
        assert_eq!(provider_text(&list, PROVIDER_DIRECT).unwrap(), "+.kova-learned.invalid\n+.a.com\n");
        assert_eq!(provider_text(&list, PROVIDER_PROXY_IP).unwrap(), "255.255.255.255/32\n149.154.167.51/32\n");
        assert_eq!(provider_text(&list, PROVIDER_DIRECT_IP).unwrap(), "255.255.255.255/32\n");
        assert!(provider_text(&list, "other").is_none());
        list.retain(|l| !l.is_ip());
        assert!(prune(&mut list, MAX_AGE_SECS + 15));
        assert_eq!(list.iter().map(|l| l.domain.as_str()).collect::<Vec<_>>(), ["b.com"]);
    }
}
