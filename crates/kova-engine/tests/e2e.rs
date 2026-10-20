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

async fn ensure_cores() -> PathBuf {
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

    // Local Xray acting as the remote server.
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
            "outbounds": [{ "protocol": "freedom" }]
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
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill").args(["/IM", "mihomo.exe", "/F"]).output();
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
