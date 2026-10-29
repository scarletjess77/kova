//! Settings that shape the generated core configs, and the routing building blocks.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Rule,
    Global,
    Direct,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warning,
    Error,
    Silent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunStack {
    #[default]
    Mixed,
    System,
    Gvisor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TunOptions {
    pub enabled: bool,
    pub stack: TunStack,
    pub auto_route: bool,
    pub strict_route: bool,
    pub mtu: u16,
}

impl Default for TunOptions {
    fn default() -> Self {
        Self { enabled: false, stack: TunStack::Mixed, auto_route: true, strict_route: true, mtu: 9000 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsOptions {
    pub fake_ip: bool,
    /// Resolvers for proxied traffic. DoH/DoT URLs or plain IPs.
    pub nameservers: Vec<String>,
    /// Resolvers for direct traffic and for proxy server hostnames.
    pub direct_nameservers: Vec<String>,
    /// Replace the DNS section of imported Clash profiles with ours.
    pub override_profile: bool,
}

impl Default for DnsOptions {
    fn default() -> Self {
        Self {
            fake_ip: true,
            nameservers: vec!["https://1.1.1.1/dns-query".into(), "https://dns.google/dns-query".into()],
            direct_nameservers: vec!["https://common.dot.dns.yandex.net/dns-query".into(), "77.88.8.8".into()],
            override_profile: false,
        }
    }
}

/// One-click routing additions shown on the Rules page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    BlockAds,
    DirectRu,
    DirectCn,
    DirectIr,
    /// Only sites blocked in Russia go through the proxy; everything else is direct.
    /// Needs the `ru-blocked` lists from runetfreedom's geodata.
    ProxyBlockedRu,
}

impl Preset {
    /// Evaluation order. Blocked-site rules come first so a blocked `.ru` domain is not
    /// sent direct by the Russian preset.
    pub const ALL: [Preset; 5] = [Preset::ProxyBlockedRu, Preset::DirectRu, Preset::BlockAds, Preset::DirectCn, Preset::DirectIr];

    pub fn rules(self) -> Vec<Rule> {
        use RuleKind::*;
        let r = |k, v: &str, t: &str| Rule { kind: k, value: v.into(), target: t.into() };
        match self {
            Preset::BlockAds => vec![r(Geosite, "category-ads-all", "REJECT")],
            Preset::DirectRu => vec![
                r(DomainSuffix, "ru", "DIRECT"),
                r(DomainSuffix, "xn--p1ai", "DIRECT"),
                r(DomainSuffix, "su", "DIRECT"),
                r(Geosite, "category-ru", "DIRECT"),
                r(Geoip, "RU", "DIRECT"),
            ],
            Preset::DirectCn => vec![r(Geosite, "cn", "DIRECT"), r(Geoip, "CN", "DIRECT")],
            Preset::DirectIr => vec![r(DomainSuffix, "ir", "DIRECT"), r(Geosite, "category-ir", "DIRECT"), r(Geoip, "IR", "DIRECT")],
            Preset::ProxyBlockedRu => vec![r(Geosite, "ru-blocked", PROXY), r(Geoip, "ru-blocked", PROXY)],
        }
    }

    /// Geosite and GeoIP categories this preset needs in the geodata files.
    pub fn needs(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Preset::BlockAds => (&["category-ads-all"], &[]),
            Preset::DirectRu => (&["category-ru"], &["ru"]),
            Preset::DirectCn => (&["cn"], &["cn"]),
            Preset::DirectIr => (&["category-ir"], &["ir"]),
            Preset::ProxyBlockedRu => (&["ru-blocked"], &["ru-blocked"]),
        }
    }

    /// Geosite categories whose domains should be resolved by the direct resolvers.
    pub fn direct_geosites(self) -> &'static [&'static str] {
        match self {
            Preset::DirectRu => &["category-ru"],
            Preset::DirectCn => &["cn"],
            Preset::DirectIr => &["category-ir"],
            Preset::BlockAds | Preset::ProxyBlockedRu => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleKind {
    Domain,
    DomainSuffix,
    DomainKeyword,
    Geosite,
    Geoip,
    IpCidr,
    ProcessName,
    Match,
}

impl RuleKind {
    /// The name Mihomo gives this kind of rule in its logs.
    pub fn mihomo_name(self) -> &'static str {
        match self {
            RuleKind::Domain => "Domain",
            RuleKind::DomainSuffix => "DomainSuffix",
            RuleKind::DomainKeyword => "DomainKeyword",
            RuleKind::Geosite => "GeoSite",
            RuleKind::Geoip => "GeoIP",
            RuleKind::IpCidr => "IPCIDR",
            RuleKind::ProcessName => "ProcessName",
            RuleKind::Match => "Match",
        }
    }

    pub fn clash(self) -> &'static str {
        match self {
            RuleKind::Domain => "DOMAIN",
            RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
            RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
            RuleKind::Geosite => "GEOSITE",
            RuleKind::Geoip => "GEOIP",
            RuleKind::IpCidr => "IP-CIDR",
            RuleKind::ProcessName => "PROCESS-NAME",
            RuleKind::Match => "MATCH",
        }
    }
}

/// A routing rule. `target` is `DIRECT`, `REJECT` or a group/node name; [`PROXY`] is the
/// main group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub kind: RuleKind,
    #[serde(default)]
    pub value: String,
    pub target: String,
}

/// Targets every config understands, whatever its groups.
const BUILTIN_TARGETS: &[&str] = &["DIRECT", "REJECT", "REJECT-DROP", "PASS", PROXY];

impl Rule {
    /// This rule with its target replaced by [`PROXY`] when `exists` doesn't know it.
    /// Rules are stored globally but groups belong to a profile; a rule written for
    /// another profile's group must not make this profile's config invalid.
    pub fn retargeted(&self, exists: impl Fn(&str) -> bool) -> Rule {
        if BUILTIN_TARGETS.contains(&self.target.as_str()) || exists(&self.target) {
            self.clone()
        } else {
            Rule { target: PROXY.into(), ..self.clone() }
        }
    }

    pub fn to_clash(&self, proxy_group: &str) -> String {
        let country = self.target.strip_prefix(COUNTRY).map(country_group);
        let target = if self.target == PROXY { proxy_group } else { country.as_deref().unwrap_or(&self.target) };
        match self.kind {
            RuleKind::Match => format!("MATCH,{target}"),
            RuleKind::IpCidr => format!("IP-CIDR,{},{target},no-resolve", self.value),
            k => format!("{},{},{target}", k.clash(), self.value),
        }
    }
}

/// Where the GeoIP / GeoSite databases come from. Both cores read the v2ray `.dat` format.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GeoSource {
    /// MetaCubeX meta-rules-dat: v2fly categories plus every country.
    #[default]
    Default,
    /// runetfreedom/russia-v2ray-rules-dat: the default set plus Russian block lists
    /// (`ru-blocked`, `ru-available-only-inside`, ...). Updated every few hours.
    Runetfreedom,
    /// Any pair of direct download URLs.
    Custom { geoip: String, geosite: String },
}

impl GeoSource {
    /// Download URLs for `geoip.dat` and `geosite.dat`.
    pub fn urls(&self) -> (String, String) {
        let gh = |repo: &str, tag: &str| {
            (format!("https://github.com/{repo}/releases/{tag}/geoip.dat"), format!("https://github.com/{repo}/releases/{tag}/geosite.dat"))
        };
        match self {
            GeoSource::Default => gh("MetaCubeX/meta-rules-dat", "download/latest"),
            GeoSource::Runetfreedom => gh("runetfreedom/russia-v2ray-rules-dat", "latest/download"),
            GeoSource::Custom { geoip, geosite } => (geoip.trim().to_string(), geosite.trim().to_string()),
        }
    }

    /// GitHub API URL of the release that publishes the files with SHA-256 digests.
    /// `None` for custom sources, which can't be verified.
    pub fn release_api(&self) -> Option<&'static str> {
        match self {
            // MetaCubeX re-publishes a release literally tagged `latest`.
            GeoSource::Default => Some("https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/tags/latest"),
            GeoSource::Runetfreedom => Some("https://api.github.com/repos/runetfreedom/russia-v2ray-rules-dat/releases/latest"),
            GeoSource::Custom { .. } => None,
        }
    }

    /// Short stable id, used to notice that a cached copy came from another source.
    pub fn id(&self) -> String {
        match self {
            GeoSource::Default => "default".into(),
            GeoSource::Runetfreedom => "runetfreedom".into(),
            GeoSource::Custom { geoip, geosite } => format!("custom:{geoip}|{geosite}"),
        }
    }
}

/// Names of the groups Kova generates for link-based profiles.
pub const PROXY: &str = "Proxy";
/// A rule target `country:US` sends traffic through the fastest of the profile's servers
/// in that country; without any, through the main group.
pub const COUNTRY: &str = "country:";

/// The group Kova makes for a country target: "US · fastest". No flag emoji, which
/// Windows draws as two letters.
pub fn country_group(code: &str) -> String {
    format!("{code} · fastest")
}

/// Country names servers are often called by, in English and Russian (as in the UI).
const COUNTRY_NAMES: &[(&str, &str)] = &[
    ("netherlands", "NL"), ("нидерланды", "NL"), ("germany", "DE"), ("германия", "DE"), ("finland", "FI"), ("финляндия", "FI"),
    ("poland", "PL"), ("польша", "PL"), ("sweden", "SE"), ("швеция", "SE"), ("united kingdom", "GB"), ("britain", "GB"), ("великобритания", "GB"),
    ("turkey", "TR"), ("türkiye", "TR"), ("турция", "TR"), ("united states", "US"), ("usa", "US"), ("сша", "US"), ("japan", "JP"), ("япония", "JP"),
    ("singapore", "SG"), ("сингапур", "SG"), ("france", "FR"), ("франция", "FR"), ("estonia", "EE"), ("эстония", "EE"), ("latvia", "LV"), ("латвия", "LV"),
    ("lithuania", "LT"), ("литва", "LT"), ("kazakhstan", "KZ"), ("казахстан", "KZ"), ("russia", "RU"), ("россия", "RU"), ("canada", "CA"), ("канада", "CA"),
    ("switzerland", "CH"), ("швейцария", "CH"), ("austria", "AT"), ("австрия", "AT"), ("hong kong", "HK"), ("гонконг", "HK"),
];

/// A server's country from its name: a flag emoji (🇳🇱), a leading code ("NL Amsterdam",
/// "NL-01") or a country name. Matches `countryCode` in the UI.
pub fn country_of(name: &str) -> Option<String> {
    let flag: Vec<u32> = name.chars().map(|c| c as u32).filter(|c| (0x1F1E6..=0x1F1FF).contains(c)).collect();
    if flag.len() >= 2 {
        return Some(flag[..2].iter().map(|c| char::from(b'A' + (c - 0x1F1E6) as u8)).collect());
    }
    let t = name.trim();
    let lead: String = t.chars().take(2).collect();
    let next = t.chars().nth(2);
    if lead.chars().count() == 2 && lead.chars().all(|c| c.is_ascii_uppercase()) && next.is_none_or(|c| c.is_whitespace() || c.is_ascii_digit() || "-_|·.".contains(c)) {
        return Some(lead);
    }
    let lower = name.to_lowercase();
    COUNTRY_NAMES.iter().find(|(k, _)| lower.contains(k)).map(|(_, v)| v.to_string())
}
pub const AUTO: &str = "Auto";
pub const FALLBACK: &str = "Fallback";
pub const TEST_URL: &str = "https://www.gstatic.com/generate_204";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeOptions {
    pub mixed_port: u16,
    /// Mihomo controller port. The engine picks a free one for every run; this value is
    /// only what the generators write.
    pub controller_port: u16,
    /// Bearer secret for the Mihomo controller; regenerated per install.
    pub secret: String,
    /// Password of [`LOCAL_USER`] on the local ports only Kova itself uses (smart
    /// routing's probes, the Xray bridge) and, on Android, on the proxy port, which every
    /// app on the phone could otherwise use. Made once per install.
    pub local_password: String,
    pub allow_lan: bool,
    pub ipv6: bool,
    pub mode: Mode,
    pub log_level: LogLevel,
    pub sniff: bool,
    pub tun: TunOptions,
    pub dns: DnsOptions,
    pub presets: Vec<Preset>,
    /// User rules, evaluated before presets.
    pub custom_rules: Vec<Rule>,
    /// Add presets and custom rules in front of an imported Clash profile's own rules.
    pub apply_rules_to_clash_profiles: bool,
    /// Port Xray exposes its metrics (traffic counters, observatory) on. Picked per run.
    pub xray_metrics_port: u16,
    pub geodata: GeoSource,
    /// Processes whose traffic always goes direct, e.g. Kova's own Xray bridge. Set per
    /// run by the engine; never stored.
    #[serde(skip)]
    pub bypass_processes: Vec<String>,
    /// Destination addresses (CIDR) kept out of the TUN routes, e.g. the servers the Xray
    /// bridge talks to. Unlike rules this also holds in Global mode. Set per run.
    #[serde(skip)]
    pub tun_exclude: Vec<String>,
    /// Learn sites that fail on their usual path and route them the other way.
    pub smart_routing: bool,
    /// Let smart routing also correct an imported Clash profile's own rules, not just
    /// traffic that only the catch-all matched. Learned sites then come before the
    /// profile's rules.
    pub smart_profile_rules: bool,
    /// Let smart routing check sites that fail through the proxy directly, and send
    /// those that work direct. Off by default: the direct check and the route after it
    /// show the user's own address to the site.
    pub smart_learn_direct: bool,
    /// Where this run's smart routing lives. Set per run when it is on.
    #[serde(skip)]
    pub smart: Option<SmartRuntime>,
    /// Android: the VPN interface's file descriptor, which the core uses as its TUN
    /// device instead of creating one. Set per run.
    #[serde(skip)]
    pub tun_fd: Option<i32>,
    /// Run without the routing data (geoip.dat, geosite.dat), which could not be had:
    /// every rule that needs it is left out and everything else goes through the proxy,
    /// through which the data is then fetched. Set per run.
    #[serde(skip)]
    pub without_geodata: bool,
    /// Beta. Make connections to the servers harder for DPI to recognise: the TLS
    /// handshake goes out in small pieces and UDP connections start with random packets.
    /// Done by a local Xray that every server connection is dialled through.
    pub anti_dpi: bool,
    /// Local port of that Xray, set per run when Anti-DPI is on and Xray is installed.
    #[serde(skip)]
    pub anti_dpi_port: Option<u16>,
    /// The installed Xray is older than v26.3.23, which brought Finalmask's fragment and
    /// noise: Anti-DPI then uses the Direct outbound's own `fragment` and `noises`, which
    /// newer Xray only warns about. Set per run.
    #[serde(skip)]
    pub xray_legacy_masks: bool,
    /// SHA-256 of the certificate of each server (by name) whose link turns certificate
    /// checks off, for Xray to pin: Xray no longer accepts "allowInsecure". Set per run.
    #[serde(skip)]
    pub cert_pins: std::collections::HashMap<String, String>,
}

/// User name on the local ports only Kova uses; see [`RuntimeOptions::local_password`].
pub const LOCAL_USER: &str = "kova";

/// Per-run parts of smart routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartRuntime {
    /// Base URL Mihomo fetches the learned rule lists from, ending in `/`.
    pub lists_url: String,
    /// Local ports that skip the rules and always go direct, or always use the main
    /// group; Kova checks a site both ways through them.
    pub direct_port: u16,
    pub proxy_port: u16,
    /// Password of [`LOCAL_USER`] on the probe ports; empty for none.
    pub password: String,
    /// Which failures to act on.
    pub scope: crate::smart::Scope,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            mixed_port: 7890,
            controller_port: 9097,
            secret: String::new(),
            local_password: String::new(),
            allow_lan: false,
            ipv6: true,
            mode: Mode::Rule,
            log_level: LogLevel::Info,
            sniff: true,
            tun: TunOptions::default(),
            dns: DnsOptions::default(),
            presets: vec![Preset::DirectRu, Preset::BlockAds],
            custom_rules: Vec::new(),
            apply_rules_to_clash_profiles: true,
            xray_metrics_port: 9098,
            geodata: GeoSource::Default,
            bypass_processes: Vec::new(),
            tun_exclude: Vec::new(),
            smart_routing: false,
            smart_profile_rules: false,
            smart_learn_direct: false,
            smart: None,
            tun_fd: None,
            without_geodata: false,
            anti_dpi: false,
            anti_dpi_port: None,
            xray_legacy_masks: false,
            cert_pins: Default::default(),
        }
    }
}

impl RuntimeOptions {
    /// Custom rules, then presets, then LAN, then the catch-all.
    pub fn rules(&self) -> Vec<Rule> {
        let mut out = self.custom_rules.clone();
        for p in Preset::ALL.iter().filter(|p| self.presets.contains(p)) {
            out.extend(p.rules());
        }
        out.retain(|r| self.can_use(r));
        out
    }

    /// LAN goes direct, then the catch-all: the main group, or DIRECT when only blocked
    /// sites should be proxied (which needs the routing data to tell what is blocked).
    pub fn tail_rules(&self) -> Vec<Rule> {
        let catch_all = if self.presets.contains(&Preset::ProxyBlockedRu) && !self.without_geodata { "DIRECT" } else { PROXY };
        let mut out = vec![
            Rule { kind: RuleKind::Geosite, value: "private".into(), target: "DIRECT".into() },
            Rule { kind: RuleKind::IpCidr, value: "10.0.0.0/8".into(), target: "DIRECT".into() },
            Rule { kind: RuleKind::IpCidr, value: "172.16.0.0/12".into(), target: "DIRECT".into() },
            Rule { kind: RuleKind::IpCidr, value: "192.168.0.0/16".into(), target: "DIRECT".into() },
            Rule { kind: RuleKind::IpCidr, value: "127.0.0.0/8".into(), target: "DIRECT".into() },
            Rule { kind: RuleKind::Match, value: String::new(), target: catch_all.into() },
        ];
        out.retain(|r| self.can_use(r));
        out
    }

    /// Whether a rule can work in this run: rules on GeoSite and GeoIP lists need the
    /// routing data.
    pub fn can_use(&self, r: &Rule) -> bool {
        !(self.without_geodata && matches!(r.kind, RuleKind::Geosite | RuleKind::Geoip))
    }

    /// Geodata categories the enabled presets need, as (geosite, geoip).
    pub fn needed_categories(&self) -> (Vec<&'static str>, Vec<&'static str>) {
        let mut site = vec!["private"];
        let mut ip = Vec::new();
        for p in &self.presets {
            let (s, i) = p.needs();
            site.extend_from_slice(s);
            ip.extend_from_slice(i);
        }
        (site, ip)
    }

    pub fn listen_addr(&self) -> &'static str {
        if self.allow_lan { "0.0.0.0" } else { "127.0.0.1" }
    }
}
