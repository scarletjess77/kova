//! Downloads real routing data. Run with:
//! `cargo test -p kova-engine --test geodata -- --ignored --nocapture`

use kova_core::options::{GeoSource, Preset, RuntimeOptions};
use kova_engine::geodata;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads ~90 MB from GitHub"]
async fn runetfreedom_is_verified_and_has_the_blocked_lists() {
    let dir = tempfile::tempdir().unwrap();
    let marker = geodata::update(dir.path(), &GeoSource::Runetfreedom, None).await.unwrap();
    assert!(marker.verified, "release digests should be checked");
    assert_eq!(marker.source, "runetfreedom");

    let opts = RuntimeOptions { presets: vec![Preset::ProxyBlockedRu, Preset::DirectRu, Preset::BlockAds], ..Default::default() };
    let (site, ip) = opts.needed_categories();
    let info = geodata::info(dir.path(), &site, &ip);
    eprintln!("{:?}", info.files);
    assert_eq!(info.files.len(), 2);
    assert!(info.missing.is_empty(), "missing: {:?}", info.missing);

    // Switching away and back reuses the cached copy instead of downloading again.
    let (_, downloaded) = geodata::switch_to(dir.path(), &GeoSource::Default, None).await.unwrap();
    assert!(downloaded, "first use of the standard source downloads it");
    let started = std::time::Instant::now();
    let (m, downloaded) = geodata::switch_to(dir.path(), &GeoSource::Runetfreedom, None).await.unwrap();
    assert!(!downloaded, "switching back must not download again");
    assert_eq!(m.source, "runetfreedom");
    eprintln!("switched back in {:?}", started.elapsed());
    assert_eq!(geodata::info(dir.path(), &site, &ip).files[1].size, info.files[1].size);

    // A custom source pointing at something that isn't geodata is rejected, and the
    // verified files stay in place.
    let bad = GeoSource::Custom { geoip: "https://example.com/".into(), geosite: "https://example.com/".into() };
    assert!(geodata::update(dir.path(), &bad, None).await.is_err());
    assert_eq!(geodata::marker(dir.path()).unwrap().source, "runetfreedom");
}
