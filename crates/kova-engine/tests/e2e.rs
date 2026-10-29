//! End-to-end: real Mihomo and Xray binaries, a local Xray server, real traffic.
//!
//! Downloads both cores from GitHub on first run (cached in `target/test-cores`).
//! Run with: `cargo test -p kova-engine --test e2e -- --ignored --nocapture`

use std::path::PathBuf;
use std::time::Duration;

use kova_core::CoreKind;
use kova_core::options::{PROXY, Rule, RuleKind};
use kova_engine::{Engine, cores};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn cache_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-cores");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A free port whose next one is free too: Xray in front also listens on the port above
/// the proxy port. Taken from below the range the system hands out for outgoing
/// connections (49152+ on Windows, 32768+ on Linux), where the other tests' connections
/// could take the second port before Xray does.
fn free_port_pair() -> u16 {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u16;
    (0..6000u16)
        .map(|i| 20000 + (seed.wrapping_add(i * 2)) % 12000)
        .find(|&p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() && std::net::TcpListener::bind(("127.0.0.1", p + 1)).is_ok())
        .expect("two free ports in a row")
}

async fn ensure_cores() -> PathBuf {
    // The tests run in parallel and share the cache: one download at a time.
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE_AT_A_TIME.lock().await;
    let dir = cache_dir();
    for core in [CoreKind::Mihomo, CoreKind::Xray] {
        if !cores::binary_path(&dir, core).exists() {
            let tag = cores::install_latest(&dir, core).await.expect("install core");
            eprintln!("installed {} {tag}", core.label());
        }
    }
    dir
}

/// A tiny HTTP server that answers every request with `hello from target`.
async fn target_server() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let body = "hello from target";
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn connects_through_ws_and_bridged_xhttp() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let status = engine.core_status(false).await;
    eprintln!("cores: {status:?}");
    assert!(status.iter().all(|s| s.installed.is_some()));

    // Local Xray acting as the remote server. From Xray 26 a server refuses private and
    // loopback targets unless `finalRules` allows them; every test server here does, as
    // the test targets run on this machine.
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let (ws_port, xh_port) = (free_port(), free_port());
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "inbounds": [
                { "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                  "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } },
                { "listen": "127.0.0.1", "port": xh_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                  "streamSettings": { "network": "xhttp", "xhttpSettings": { "path": "/x" } } }
            ],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray))
        .args(["run", "-c", server_cfg.to_str().unwrap()])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(server.try_wait().unwrap().is_none(), "test server exited");

    let target = target_server().await;

    // Settings: never touch the real system proxy in tests; send `localhost` through the proxy.
    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.controller_port = free_port();
    s.core.presets.clear();
    s.core.custom_rules = vec![Rule { kind: RuleKind::Domain, value: "localhost".into(), target: PROXY.into() }];
    engine.update_settings(s.clone()).await.unwrap();

    let links = format!(
        "vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Local WS\n\
         vless://{uuid}@127.0.0.1:{xh_port}?encryption=none&security=none&type=xhttp&path=%2Fx&mode=packet-up#Local XHTTP\n"
    );
    let profile = engine.add_profile(&links, Some("E2E".into())).await.unwrap();
    assert_eq!(profile.node_count, 2);

    // Latency test while disconnected spins up a temporary tester.
    let delays = engine.test_delays(None).await.unwrap();
    eprintln!("offline delays: {delays:?}");
    assert!(delays.get("Local WS").copied().flatten().is_some(), "WS node should answer");
    assert!(delays.get("Local XHTTP").copied().flatten().is_some(), "bridged XHTTP node should answer");

    // A busy proxy port is reported up front instead of starting a core without a listener.
    {
        let _squatter = std::net::TcpListener::bind(("127.0.0.1", s.core.mixed_port)).unwrap();
        let err = engine.start().await.expect_err("start must fail while the port is taken");
        assert!(err.to_string().contains("already used"), "unexpected error: {err}");
        assert!(!engine.status().running);
    }

    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));
    let st = engine.status();
    eprintln!("status: {st:?}");
    assert!(st.running);
    assert_eq!(st.core, Some(CoreKind::Mihomo));
    assert!(st.sidecar, "XHTTP node should be bridged through Xray");
    assert_eq!(st.bridged, vec!["Local XHTTP".to_string()]);

    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.core.mixed_port)).unwrap()).timeout(Duration::from_secs(10)).build().unwrap();
    for node in ["Local WS", "Local XHTTP"] {
        engine.select(PROXY, node).await.unwrap();
        let body = client.get(format!("http://localhost:{target}/")).send().await.unwrap_or_else(|e| panic!("request via {node} failed: {e}")).text().await.unwrap();
        assert_eq!(body, "hello from target", "via {node}");
        eprintln!("ok via {node}");
    }

    let snap = engine.proxies().await.unwrap();
    let proxy_group = snap.groups.iter().find(|g| g.name == PROXY).unwrap();
    assert_eq!(proxy_group.now, "Local XHTTP");
    // Global mode must go through the main group, not Mihomo's default DIRECT.
    let global = snap.groups.iter().find(|g| g.name == "GLOBAL").unwrap();
    assert_eq!(global.now, PROXY);

    // A settings save from the UI carries an old copy of the selections; it must not win.
    let mut stale = s.clone();
    stale.selections.clear();
    stale.core.mode = kova_core::options::Mode::Global;
    engine.update_settings(stale).await.unwrap();
    assert!(!engine.settings().selections.is_empty(), "selections were overwritten by a stale save");

    // If the core dies on its own, the engine notices and reports it (hybrid mode included).
    // Only the Mihomo this test started is stopped: the machine may be running Kova for real.
    #[cfg(windows)]
    {
        // Started by this test process and running from this test's data folder: the
        // other tests here run in the same process.
        let dir = root.path().file_name().unwrap().to_string_lossy().to_string();
        let filter = format!("Name='mihomo.exe' AND ParentProcessId={} AND CommandLine LIKE '%{dir}%'", std::process::id());
        let script = format!("Get-CimInstance Win32_Process -Filter \"{filter}\" | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}");
        let _ = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();
        let mut noticed = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let st = engine.status();
            if !st.running && st.error.is_some() {
                noticed = true;
                break;
            }
        }
        assert!(noticed, "a crashed core was not reported: {:?}", engine.status());
    }

    engine.stop().await.unwrap();
    assert!(!engine.status().running);
    let _ = server.kill().await;
}

/// Like [`target_server`], but answers after a pause, so the connection can be looked
/// up in Mihomo while it is open.
async fn slow_server() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow").await;
            });
        }
    });
    port
}

/// The running Mihomo's controller, read from the config the engine wrote.
struct Controller {
    base: String,
    secret: String,
    config: serde_yaml_ng::Value,
}

impl Controller {
    fn read(root: &std::path::Path) -> Self {
        let text = std::fs::read_to_string(root.join("runtime/main/mihomo/config.yaml")).unwrap();
        let config: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
        let base = format!("http://{}", config["external-controller"].as_str().unwrap());
        Self { base, secret: config["secret"].as_str().unwrap().to_string(), config }
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let c = reqwest::Client::builder().no_proxy().build().unwrap();
        c.get(format!("{}{path}", self.base)).bearer_auth(&self.secret).send().await.unwrap().json().await.unwrap()
    }

    fn listener_port(&self, name: &str) -> u16 {
        let l = self.config["listeners"].as_sequence().unwrap().iter().find(|l| l["name"].as_str() == Some(name)).unwrap();
        l["port"].as_u64().unwrap() as u16
    }

    async fn rule_count(&self, provider: &str) -> u64 {
        self.get("/providers/rules").await["providers"][provider]["ruleCount"].as_u64().unwrap_or(0)
    }

    /// Fetch `url` through local port `via` and report the rule and chain Mihomo used.
    /// Fetch `url` through the local port `via` (with Kova's login when `password` isn't
    /// empty) and see which rule and chain Mihomo used.
    async fn route_of(&self, url: &str, via: u16, password: &str) -> (String, Vec<String>) {
        let proxy = if password.is_empty() { format!("http://127.0.0.1:{via}") } else { format!("http://kova:{password}@127.0.0.1:{via}") };
        let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(proxy).unwrap()).build().unwrap();
        let req = tokio::spawn({
            let url = url.to_string();
            async move { client.get(url).send().await.unwrap().text().await.unwrap() }
        });
        // Wait for the connection to show up: with the other tests' cores running at the
        // same time it can take a while.
        let port = url.trim_end_matches('/').rsplit(':').next().unwrap().to_string();
        let mut open = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let conns = self.get("/connections").await;
            open = conns["connections"].as_array().unwrap().iter().find(|c| c["metadata"]["destinationPort"].as_str() == Some(port.as_str())).cloned();
            if open.is_some() {
                break;
            }
        }
        let c = open.expect("connection is open");
        assert_eq!(req.await.unwrap(), "slow");
        let chains = c["chains"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        (c["rule"].as_str().unwrap_or_default().to_string(), chains)
    }
}

/// Smart routing against the real Mihomo: the config with learned lists and probe
/// ports is accepted, learned sites apply and go away without a restart, the probe
/// ports skip the rules, and a proxy that is down teaches nothing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn smart_routing_in_real_mihomo() {
    use kova_core::smart::{PROVIDER_PROXY, PROVIDER_PROXY_IP, parse_dial_failure, parse_opened};
    use kova_engine::smart::reaches;

    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    // A site learned in an earlier session.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let learned = serde_json::json!([{ "domain": "localhost", "route": "proxy", "reason": "reset", "learned_at": now }]);
    std::fs::write(root.path().join("learned.json"), learned.to_string()).unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }

    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                           "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray)).args(["run", "-c", server_cfg.to_str().unwrap()]).kill_on_drop(true).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let slow = slow_server().await;

    // Never touch the real system proxy; nothing in this test enables TUN.
    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.presets.clear();
    s.core.custom_rules.clear();
    s.core.smart_routing = true;
    engine.update_settings(s.clone()).await.unwrap();
    let dead = free_port();
    let links = format!(
        "vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Local WS\n\
         vless://{uuid}@127.0.0.1:{dead}?encryption=none&security=none&type=ws&path=%2Fws#Dead\n"
    );
    engine.add_profile(&links, Some("Smart".into())).await.unwrap();
    let mut events = engine.subscribe();
    engine.start().await.unwrap_or_else(|e| panic!("Mihomo rejected the smart-routing config: {e}"));
    engine.select(PROXY, "Local WS").await.unwrap();
    let ctl = Controller::read(root.path());
    let url = format!("http://localhost:{slow}/");

    // The learned list reached Mihomo (one line is a placeholder) and routes the site
    // through the proxy, where without it `localhost` would go direct.
    let mut count = 0;
    for _ in 0..20 {
        count = ctl.rule_count(PROVIDER_PROXY).await;
        if count == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(count, 2, "learned list not loaded");
    let (rule, chains) = ctl.route_of(&url, s.core.mixed_port, "").await;
    assert_eq!(rule, "RuleSet");
    assert!(chains.contains(&"Local WS".to_string()), "{chains:?}");

    // Forgetting it applies at once, without restarting the core.
    let since = engine.status().since;
    engine.forget_learned("localhost").await.unwrap();
    assert_eq!(ctl.rule_count(PROVIDER_PROXY).await, 1);
    let (rule, chains) = ctl.route_of(&url, s.core.mixed_port, "").await;
    assert_ne!(rule, "RuleSet");
    assert_eq!(chains, ["DIRECT"]);
    assert_eq!(engine.status().since, since, "the core was restarted");

    // Every opened connection is logged with the rule that chose its route; spotting
    // resets right after the greeting depends on reading these lines.
    let mut opened = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while opened.is_none() && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(kova_engine::Event::Log { source, message, .. })) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            if source == "mihomo" {
                opened = message.find("[TCP] ").and_then(|i| parse_opened(&message[i..])).filter(|o| o.host == "localhost" && o.direct);
            }
        }
    }
    let o = opened.expect("Mihomo's line for an opened connection was not recognised");
    assert_eq!(o.port, slow);
    assert_ne!(o.rule, "RuleSet(kova-learned-proxy)", "the forgotten site still matched the learned list");

    // The probe ports ignore the rules: one always uses the proxy, one always goes direct.
    // Only Kova may use them.
    let password = engine.settings().core.local_password;
    assert!(!password.is_empty());
    let (_, chains) = ctl.route_of(&url, ctl.listener_port("kova-probe-proxy"), &password).await;
    assert!(chains.contains(&"Local WS".to_string()), "{chains:?}");
    let (_, chains) = ctl.route_of(&url, ctl.listener_port("kova-probe-direct"), &password).await;
    assert_eq!(chains, ["DIRECT"]);
    let anyone = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", ctl.listener_port("kova-probe-direct"))).unwrap()).build().unwrap();
    assert_eq!(anyone.get(&url).send().await.unwrap().status(), 407, "a probe port let in a program without the login");

    // Bare addresses (apps that connect without a name, as in TUN mode) are checked at
    // the connection level through the same ports. Their list is loaded too.
    assert_eq!(ctl.rule_count(PROVIDER_PROXY_IP).await, 1);
    let open: std::net::SocketAddr = format!("127.0.0.1:{slow}").parse().unwrap();
    let closed: std::net::SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    for name in ["kova-probe-direct", "kova-probe-proxy"] {
        let port = ctl.listener_port(name);
        assert!(reaches(port, &password, open).await, "{name}: an open port looked closed");
        // Through VLESS the server reports a failed connection only after accepting it.
        assert!(!reaches(port, &password, closed).await, "{name}: a closed port looked open");
    }

    // With the proxy down every site fails through it. That must teach nothing, or every
    // site would end up "refusing the proxy". The warning Mihomo prints for the failure
    // is the one the learner reads.
    engine.select(PROXY, "Dead").await.unwrap();
    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.core.mixed_port)).unwrap()).timeout(Duration::from_secs(5)).build().unwrap();
    let _ = client.get("http://example.com/").send().await;
    let mut parsed = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while parsed.is_none() && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(kova_engine::Event::Log { source, message, .. })) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            if source == "mihomo" {
                parsed = message.find("[TCP] dial").and_then(|i| parse_dial_failure(&message[i..]));
            }
        }
    }
    let f = parsed.expect("Mihomo's dial warning was not recognised");
    assert!(f.rule == "Match" && !f.direct && f.host == "example.com", "{f:?}");
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(engine.learned_sites().is_empty(), "learned from a dead proxy: {:?}", engine.learned_sites());

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}

/// A chosen node that stops answering is swapped for one that works, after two failed
/// checks in a row, and the chosen node stays when it answers.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn switches_away_from_a_dead_node() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                           "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray))
        .args(["run", "-c", server_cfg.to_str().unwrap()])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.controller_port = free_port();
    s.core.presets.clear();
    engine.update_settings(s).await.unwrap();
    let dead = free_port();
    let links = format!(
        "vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Live\n\
         vless://{uuid}@127.0.0.1:{dead}?encryption=none&security=none&type=ws&path=%2Fws#Dead\n"
    );
    engine.add_profile(&links, Some("Switch".into())).await.unwrap();
    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));

    engine.select(PROXY, "Live").await.unwrap();
    assert_eq!(engine.check_node().await, None, "a working node was replaced");

    // Diagnostics on a working connection: every step passes except routing apps, as
    // tests never set the system proxy.
    let steps = engine.diagnose().await;
    let state = |id: &str| steps.iter().find(|c| c.id == id).map(|c| c.state).unwrap_or_else(|| panic!("no {id} step"));
    use kova_engine::diag::State;
    for id in ["profile", "cores", "internet", "running", "node", "dns", "through"] {
        assert_eq!(state(id), State::Ok, "{id}: {steps:?}");
    }
    assert_eq!(state("capture"), State::Warn);
    engine.select(PROXY, "Dead").await.unwrap();
    assert_eq!(engine.check_node().await, None, "one failed check is not enough");
    let steps = engine.diagnose().await;
    let node = steps.iter().find(|c| c.id == "node").unwrap();
    assert_eq!((node.state, node.fix), (kova_engine::diag::State::Fail, Some("switch-node")), "{steps:?}");
    assert_eq!(engine.check_node().await, Some(("Dead".to_string(), "Live".to_string())));
    assert_eq!(engine.current_node().await.as_deref(), Some("Live"));
    // For this connection only: the user's own choice stays saved.
    let key = format!("{}/{PROXY}", engine.settings().active_profile.unwrap());
    assert_eq!(engine.settings().selections.get(&key).map(String::as_str), Some("Dead"));

    let mut s = engine.settings();
    s.auto_switch = false;
    engine.update_settings(s).await.unwrap();
    engine.select(PROXY, "Dead").await.unwrap();
    for _ in 0..3 {
        assert_eq!(engine.check_node().await, None, "switched with the option off");
    }

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}

/// A first start where the routing data can't be downloaded directly (GitHub blocked):
/// Kova connects without it, fetches it through the server, and reconnects with every
/// rule. Here the data lives at a name only the server can resolve.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn first_start_gets_routing_data_through_the_server() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        let is_data = e.path().extension().is_some_and(|x| x == "dat");
        if e.path().is_file() && !is_data {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    assert!(!engine.paths.cores_dir().join("geosite.dat").exists());

    // The routing data, served locally.
    let files = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let files_port = files.local_addr().unwrap().port();
    let cache = cores_cache.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = files.accept().await else { return };
            let cache = cache.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let name = if head.contains("geoip.dat") { "geoip.dat" } else { "geosite.dat" };
                let body = std::fs::read(cache.join(name)).unwrap();
                let crlf = "\r\n";
                let _ = s.write_all(format!("HTTP/1.1 200 OK{crlf}Content-Length: {}{crlf}Connection: close{crlf}{crlf}", body.len()).as_bytes()).await;
                let _ = s.write_all(&body).await;
            });
        }
    });

    // The server resolves "geodata.test" for its own outbound; nothing else can.
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "dns": { "hosts": { "geodata.test": "127.0.0.1" } },
            "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                           "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "domainStrategy": "UseIP", "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray))
        .args(["run", "-c", server_cfg.to_str().unwrap()])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.controller_port = free_port();
    s.core.presets.clear();
    s.core.geodata = kova_core::options::GeoSource::Custom {
        geoip: format!("http://geodata.test:{files_port}/geoip.dat"),
        geosite: format!("http://geodata.test:{files_port}/geosite.dat"),
    };
    // A new source is downloaded right away, which fails here like it would where GitHub
    // is blocked; the setting is saved first all the same.
    let _ = engine.update_settings(s).await;
    assert!(matches!(engine.settings().core.geodata, kova_core::options::GeoSource::Custom { .. }));
    engine.add_profile(&format!("vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Live\n"), Some("Data".into())).await.unwrap();

    let mut events = engine.subscribe();
    engine.start().await.unwrap_or_else(|e| panic!("could not connect without the routing data: {e}"));
    let st = engine.status();
    assert!(st.running && st.warning.as_deref().is_some_and(|w| w.contains("without the routing data")), "{st:?}");

    // Fetched through the server, then reconnected with every rule.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut noticed = false;
    while !noticed && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(kova_engine::Event::Notice { .. })) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            noticed = true;
        }
    }
    assert!(noticed, "the routing data never arrived through the server");
    assert!(engine.paths.cores_dir().join("geosite.dat").exists());
    let st = engine.status();
    assert!(st.running && st.warning.is_none(), "{st:?}");
    let config = std::fs::read_to_string(engine.paths.runtime_dir().join("main").join("mihomo").join("config.yaml")).unwrap();
    assert!(config.contains("GEOSITE,private,DIRECT"), "the full rules are back");

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}

/// Anti-DPI (beta): every server connection goes through the local Xray, which sends the
/// TLS handshake in small pieces. The server here speaks VLESS over TLS, so the pieces
/// must reassemble into a handshake it accepts, and pages must still load.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn anti_dpi_carries_tls_traffic() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let xray = cores::binary_path(&cores_cache, CoreKind::Xray);
    let cert = std::process::Command::new(&xray).args(["tls", "cert", "-domain=localhost", "-json"]).output().unwrap();
    let cert: serde_json::Value = serde_json::from_slice(&cert.stdout).expect("a certificate");
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let tls_port = free_port();
    let target = target_server().await;
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "inbounds": [{ "listen": "127.0.0.1", "port": tls_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                           "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" }, "security": "tls",
                                               "tlsSettings": { "certificates": [{ "certificate": cert["certificate"], "key": cert["key"] }] } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(&xray).args(["run", "-c", server_cfg.to_str().unwrap()]).kill_on_drop(true).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port_pair();
    s.core.presets.clear();
    s.core.anti_dpi = true;
    engine.update_settings(s.clone()).await.unwrap();
    let link = format!("vless://{uuid}@127.0.0.1:{tls_port}?encryption=none&security=tls&sni=localhost&allowInsecure=1&type=ws&path=%2Fws#TLS");
    let profile = engine.add_profile(&link, Some("Anti-DPI".into())).await.unwrap();
    // Latency tests while disconnected go through Anti-DPI too.
    let delays = engine.test_delays(None).await.unwrap();
    assert!(delays.get("TLS").copied().flatten().is_some(), "offline latency through Anti-DPI: {delays:?}");
    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));
    assert!(engine.status().sidecar, "Anti-DPI runs in the Xray sidecar");

    // The server is dialled through the anti-DPI hop.
    let cfg = std::fs::read_to_string(root.path().join("runtime/main/mihomo/config.yaml")).unwrap();
    assert!(cfg.contains("dialer-proxy: Kova anti-DPI"), "{cfg}");

    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.core.mixed_port)).unwrap()).timeout(Duration::from_secs(10)).build().unwrap();
    let body = client.get(format!("http://localhost:{target}/")).send().await.expect("request through Anti-DPI").text().await.unwrap();
    assert_eq!(body, "hello from target");

    // A profile pinned to Xray: Xray itself dials the server through its anti-DPI outbound.
    let patch = kova_engine::engine::ProfilePatch { core: Some(kova_core::CorePreference::Xray), ..Default::default() };
    engine.edit_profile(&profile.id, patch).await.unwrap();
    engine.restart().await.unwrap_or_else(|e| panic!("restart with Xray failed: {e}"));
    assert_eq!(engine.status().core, Some(CoreKind::Xray));
    let cfg = std::fs::read_to_string(root.path().join("runtime/main/xray.json")).unwrap();
    assert!(cfg.contains("\"dialerProxy\": \"anti-dpi\""), "{cfg}");
    let body = client.get(format!("http://localhost:{target}/")).send().await.expect("request through Xray's Anti-DPI").text().await.unwrap();
    assert_eq!(body, "hello from target");

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}

/// Anti-DPI with a UDP protocol: Hysteria2 (QUIC) dialled through the local Xray's UDP.
/// The server is a Mihomo listener. First without Anti-DPI, as the baseline.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn anti_dpi_carries_hysteria2() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let xray = cores::binary_path(&cores_cache, CoreKind::Xray);
    let cert = std::process::Command::new(&xray).args(["tls", "cert", "-domain=localhost", "-json"]).output().unwrap();
    let cert: serde_json::Value = serde_json::from_slice(&cert.stdout).expect("a certificate");
    let pem = |k: &str| cert[k].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect::<Vec<_>>().join("\n") + "\n";
    let server_dir = tempfile::tempdir().unwrap();
    std::fs::write(server_dir.path().join("cert.pem"), pem("certificate")).unwrap();
    std::fs::write(server_dir.path().join("key.pem"), pem("key")).unwrap();
    let hy_port = free_port();
    let target = target_server().await;
    std::fs::write(
        server_dir.path().join("config.yaml"),
        format!(
            "mixed-port: 0\nmode: direct\nlog-level: warning\nlisteners:\n  - name: hy2\n    type: hysteria2\n    listen: 127.0.0.1\n    port: {hy_port}\n    users:\n      kova: testpass\n    certificate: ./cert.pem\n    private-key: ./key.pem\n"
        ),
    )
    .unwrap();
    let dir = server_dir.path().to_str().unwrap().to_string();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Mihomo))
        .args(["-d", &dir, "-f", &format!("{dir}/config.yaml")])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.presets.clear();
    engine.update_settings(s.clone()).await.unwrap();
    engine.add_profile(&format!("hysteria2://testpass@127.0.0.1:{hy_port}?insecure=1&sni=localhost#HY2"), Some("UDP".into())).await.unwrap();
    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.core.mixed_port)).unwrap()).timeout(Duration::from_secs(10)).build().unwrap();

    for anti_dpi in [false, true] {
        let mut s = engine.settings();
        s.core.anti_dpi = anti_dpi;
        engine.update_settings(s).await.unwrap();
        engine.start().await.unwrap_or_else(|e| panic!("start failed (anti-DPI {anti_dpi}): {e}"));
        let body = client.get(format!("http://localhost:{target}/")).send().await.unwrap_or_else(|e| panic!("Hysteria2 with anti-DPI {anti_dpi}: {e}")).text().await.unwrap();
        assert_eq!(body, "hello from target", "anti-DPI {anti_dpi}");
        engine.stop().await.unwrap();
    }
    let _ = server.kill().await;
}

/// Android: on mobile data Kova uses the server set for it, and on Wi-Fi the usual
/// choice again, without overwriting that choice.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn mobile_data_uses_its_own_server() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                           "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray)).args(["run", "-c", server_cfg.to_str().unwrap()]).kill_on_drop(true).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.presets.clear();
    s.mobile_server = Some("Relay".into());
    engine.update_settings(s).await.unwrap();
    let links = format!(
        "vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Home\n\
         vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Relay\n"
    );
    engine.add_profile(&links, Some("Mobile".into())).await.unwrap();
    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));
    engine.select(PROXY, "Home").await.unwrap();
    let now = |e: &std::sync::Arc<Engine>| {
        let e = e.clone();
        async move { e.proxies().await.unwrap().groups.into_iter().find(|g| g.name == PROXY).unwrap().now }
    };

    engine.network_changed(true).await;
    assert_eq!(now(&engine).await, "Relay", "mobile data uses its own server");
    engine.network_changed(false).await;
    assert_eq!(now(&engine).await, "Home", "Wi-Fi goes back to the usual choice");
    assert_eq!(engine.settings().selections.values().next().map(String::as_str), Some("Home"), "the usual choice is kept");

    // Connecting while on mobile data starts on its server.
    engine.network_changed(true).await;
    engine.restart().await.unwrap();
    assert_eq!(now(&engine).await, "Relay");

    // Turning the mobile-data server off in Settings while on mobile data: the usual
    // choice at once.
    let mut s = engine.settings();
    s.mobile_server = None;
    engine.update_settings(s).await.unwrap();
    assert_eq!(now(&engine).await, "Home");

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}

/// Kill switch (beta): when the core dies on its own, Kova reconnects by itself, and a
/// user's Disconnect ends the hold. (The system proxy part isn't exercised: tests never
/// touch the system proxy.)
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn kill_switch_reconnects_after_a_drop() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.presets.clear();
    s.kill_switch = true;
    engine.update_settings(s).await.unwrap();
    let link = format!("vless://b831381d-6324-4d53-ad4f-8cda48b30811@127.0.0.1:{}?encryption=none&security=none&type=ws&path=%2Fws#Any", free_port());
    engine.add_profile(&link, Some("Kill switch".into())).await.unwrap();
    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));

    // Stop only the Mihomo this test started (the machine may be running Kova for real).
    let dir = root.path().file_name().unwrap().to_string_lossy().to_string();
    let filter = format!("Name='mihomo.exe' AND ParentProcessId={} AND CommandLine LIKE '%{dir}%'", std::process::id());
    let script = format!("Get-CimInstance Win32_Process -Filter \"{filter}\" | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}");
    let _ = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();

    let mut dropped = false;
    let mut back = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let running = engine.status().running;
        dropped |= !running;
        if dropped && running {
            back = true;
            break;
        }
    }
    assert!(dropped, "the drop was noticed");
    assert!(back, "the kill switch reconnected: {:?}", engine.status());

    engine.stop().await.unwrap();
    assert!(!engine.status().running);
}

/// Xray in front with LAN sharing: this computer uses the proxy port without a login,
/// the network gets the two ports above it with Kova's login, and the proxy port itself
/// can't be reached from the network.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads cores and runs real processes"]
async fn xray_shares_with_the_network_only_with_the_login() {
    let cores_cache = ensure_cores().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    for entry in std::fs::read_dir(&cores_cache).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), engine.paths.cores_dir().join(e.file_name())).unwrap();
        }
    }
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let server_dir = tempfile::tempdir().unwrap();
    let server_cfg = server_dir.path().join("server.json");
    std::fs::write(
        &server_cfg,
        serde_json::json!({
            "log": { "loglevel": "warning" },
            "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
            "outbounds": [{ "protocol": "freedom", "settings": { "finalRules": [{ "action": "allow" }] } }]
        })
        .to_string(),
    )
    .unwrap();
    let mut server = tokio::process::Command::new(cores::binary_path(&cores_cache, CoreKind::Xray)).args(["run", "-c", server_cfg.to_str().unwrap()]).kill_on_drop(true).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let target = target_server().await;

    // Four free ports in a row: proxy, SOCKS above it, and the two for the network.
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u16;
    let base = (0..3000u16)
        .map(|i| 20000 + seed.wrapping_add(i * 4) % 12000)
        .find(|&p| (p..p + 4).all(|q| std::net::TcpListener::bind(("0.0.0.0", q)).is_ok()))
        .expect("four free ports in a row");
    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = base;
    s.core.allow_lan = true;
    s.core.presets.clear();
    // By address, not by name: no DNS, so a Kova running in TUN mode on this machine
    // (fake-IP answers) can't get in the way.
    s.core.custom_rules = vec![Rule { kind: RuleKind::IpCidr, value: "127.0.0.1/32".into(), target: PROXY.into() }];
    engine.update_settings(s.clone()).await.unwrap();
    let password = engine.settings().core.local_password;
    assert!(!password.is_empty());

    let link = format!("vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Local WS");
    let profile = engine.add_profile(&link, Some("Xray LAN".into())).await.unwrap();
    engine.edit_profile(&profile.id, kova_engine::engine::ProfilePatch { core: Some(kova_core::CorePreference::Xray), ..Default::default() }).await.unwrap();
    engine.start().await.unwrap_or_else(|e| panic!("start failed: {e}"));
    assert_eq!(engine.status().core, Some(CoreKind::Xray));

    let fetch = |proxy: reqwest::Proxy| async move {
        let c = reqwest::Client::builder().proxy(proxy).timeout(Duration::from_secs(10)).build().unwrap();
        c.get(format!("http://127.0.0.1:{target}/")).send().await
    };
    let url = |scheme: &str, port: u16| format!("{scheme}://127.0.0.1:{port}");

    // This computer: no login.
    let r = fetch(reqwest::Proxy::all(url("http", base)).unwrap()).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "hello from target");
    // The network's HTTP port: refused without the login, works with it.
    let r = fetch(reqwest::Proxy::all(url("http", base + 2)).unwrap()).await.unwrap();
    assert_eq!(r.status(), 407, "the LAN port must ask for the login");
    let r = fetch(reqwest::Proxy::all(url("http", base + 2)).unwrap().basic_auth("kova", &password)).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "hello from target");
    // The network's SOCKS port: the same.
    assert!(fetch(reqwest::Proxy::all(url("socks5h", base + 3)).unwrap()).await.is_err(), "SOCKS without the login");
    let r = fetch(reqwest::Proxy::all(format!("socks5h://kova:{password}@127.0.0.1:{}", base + 3)).unwrap()).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "hello from target");

    // The open proxy port listens on loopback only: not on this machine's network address.
    let lan_ip = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|u| u.connect("8.8.8.8:80").and_then(|_| u.local_addr())).map(|a| a.ip()).ok().filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
    if let Some(ip) = lan_ip {
        assert!(std::net::TcpStream::connect_timeout(&(ip, base).into(), Duration::from_secs(2)).is_err(), "proxy port reachable at {ip}");
        assert!(std::net::TcpStream::connect_timeout(&(ip, base + 2).into(), Duration::from_secs(2)).is_ok(), "LAN port not reachable at {ip}");
    } else {
        eprintln!("no network address; skipped the reachability check");
    }

    engine.stop().await.unwrap();
    let _ = server.kill().await;
}
