//! The service path end to end, without registering a Windows service: the pipe server
//! runs in-process on a private pipe, and the engine is told to use it. TUN stays off,
//! so nothing on the machine is rerouted.
//!
//! Needs the cores the engine e2e test downloads into `target/test-cores`.
//! Run with: `cargo test -p kova-service --test service_e2e -- --ignored --nocapture`
#![cfg(windows)]

use std::path::PathBuf;
use std::time::Duration;

use kova_core::options::{GeoSource, PROXY, Rule, RuleKind};
use kova_engine::Engine;
use kova_engine::service::{Request, call};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn test_cores() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-cores")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn target_server() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let body = "hello via service";
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs real cores; needs target/test-cores from the engine e2e test"]
async fn engine_runs_mihomo_through_the_service() {
    assert!(test_cores().join("mihomo.exe").exists(), "run the kova-engine e2e test first to download the cores");

    // A private service instance with Mihomo and geodata already in place (no downloads).
    let home = tempfile::tempdir().unwrap();
    let svc_cores = home.path().join("cores");
    std::fs::create_dir_all(&svc_cores).unwrap();
    for f in ["mihomo.exe", "geoip.dat", "geosite.dat"] {
        std::fs::copy(test_cores().join(f), svc_cores.join(f)).unwrap();
    }
    std::fs::write(svc_cores.join("geodata.json"), r#"{"source":"default","verified":true,"updated_at":0}"#).unwrap();
    let pipe = format!(r"\\.\pipe\kova-test-{}", std::process::id());
    // SAFETY: set before any other thread reads the environment.
    unsafe {
        std::env::set_var("KOVA_SERVICE_PIPE", &pipe);
        std::env::set_var("KOVA_FORCE_SERVICE", "1");
    }
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(kova_service::serve(pipe.clone(), home.path().to_path_buf(), async {
        let _ = stop_rx.await;
    }));
    // Wait until the service is listening.
    for _ in 0..50 {
        if call(&Request::Hello, Duration::from_secs(2)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Clients that connect and vanish, or connect and never write, must not stop it.
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        for _ in 0..5 {
            drop(ClientOptions::new().open(&pipe).unwrap());
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let _idle = loop {
            match ClientOptions::new().open(&pipe) {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(Duration::from_millis(30)).await,
            }
        };
        let hello = call(&Request::Hello, Duration::from_secs(5)).await.expect("service still answers with an idle client connected");
        assert!(hello.ok);
    }

    let hello = call(&Request::Hello, Duration::from_secs(5)).await.expect("service answers");
    eprintln!("service: {:?} core {:?}", hello.version, hello.core_version);
    assert!(hello.core_version.is_some());
    assert_eq!(hello.exe_sha256, kova_engine::service::own_exe_sha256(), "the service reports the hash of its executable");

    // An unsafe config is refused.
    let bad = call(&Request::Start { config: "external-controller: 0.0.0.0:9090\n".into(), geodata: GeoSource::Default, owner_pid: std::process::id() }, Duration::from_secs(10)).await;
    assert!(bad.is_err(), "config without a secret on a public controller must be refused");

    // Local Xray server playing the remote end.
    let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811";
    let ws_port = free_port();
    let srv_dir = tempfile::tempdir().unwrap();
    let srv_cfg = srv_dir.path().join("server.json");
    std::fs::write(&srv_cfg, serde_json::json!({
        "inbounds": [{ "listen": "127.0.0.1", "port": ws_port, "protocol": "vless", "settings": { "clients": [{ "id": uuid }], "decryption": "none" },
                       "streamSettings": { "network": "ws", "wsSettings": { "path": "/ws" } } }],
        "outbounds": [{ "protocol": "freedom" }]
    }).to_string()).unwrap();
    let mut xray = tokio::process::Command::new(test_cores().join("xray.exe")).args(["run", "-c", srv_cfg.to_str().unwrap()]).kill_on_drop(true).spawn().unwrap();
    let target = target_server().await;

    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path()).unwrap();
    let mut events = engine.subscribe();
    let mut s = engine.settings();
    s.system_proxy = false;
    s.core.mixed_port = free_port();
    s.core.presets.clear();
    s.core.custom_rules = vec![Rule { kind: RuleKind::Domain, value: "localhost".into(), target: PROXY.into() }];
    engine.update_settings(s.clone()).await.unwrap();
    engine.add_profile(&format!("vless://{uuid}@127.0.0.1:{ws_port}?encryption=none&security=none&type=ws&path=%2Fws#Svc WS"), None).await.unwrap();

    engine.start().await.unwrap_or_else(|e| panic!("start through the service failed: {e}"));
    assert!(engine.status().running);
    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.core.mixed_port)).unwrap()).build().unwrap();
    let body = client.get(format!("http://localhost:{target}/")).send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "hello via service");

    // Mihomo's log lines arrive through the service.
    let mut got_log = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && !got_log {
        if let Ok(Ok(kova_engine::Event::Log { source, .. })) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            got_log = source == "mihomo";
        }
    }
    assert!(got_log, "no Mihomo log lines were relayed from the service");

    // A crash inside the service is noticed by the app. Only the Mihomo this test's
    // in-process service started is stopped: the machine may be running Kova for real.
    {
        let filter = format!("Name='mihomo.exe' AND ParentProcessId={}", std::process::id());
        let script = format!("Get-CimInstance Win32_Process -Filter \"{filter}\" | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}");
        let _ = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();
    }
    let mut noticed = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if !engine.status().running && engine.status().error.is_some() {
            noticed = true;
            break;
        }
    }
    assert!(noticed, "core crash in the service went unnoticed: {:?}", engine.status());

    // The service stops Mihomo when the app that started it exits.
    let mut owner = std::process::Command::new("ping").args(["-n", "3", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
    let ctl = free_port();
    let cfg = format!("mixed-port: {}\nexternal-controller: 127.0.0.1:{ctl}\nsecret: 0123456789abcdef0123456789abcdef\nrules:\n  - MATCH,DIRECT\n", free_port());
    call(&Request::Start { config: cfg, geodata: GeoSource::Default, owner_pid: owner.id() }, Duration::from_secs(30)).await.unwrap();
    assert!(call(&Request::Status, Duration::from_secs(5)).await.unwrap().running);
    let _ = owner.wait();
    let mut stopped = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let st = call(&Request::Status, Duration::from_secs(5)).await.unwrap();
        if !st.running {
            eprintln!("stopped: {:?}", st.exit);
            stopped = true;
            break;
        }
    }
    assert!(stopped, "Mihomo kept running after its owner exited");

    let _ = stop_tx.send(());
    server.await.unwrap().unwrap();
    let _ = xray.kill().await;
}
