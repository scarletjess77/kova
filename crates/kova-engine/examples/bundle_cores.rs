//! Put verified Mihomo and Xray (and the routing data that comes with Xray) into a
//! folder, for the desktop installers to carry: Kova then starts without downloading
//! anything from GitHub, which is blocked in some countries.
//!
//! Usage: cargo run -p kova-engine --example bundle_cores -- <folder> [target-triple]
//! The target picks the processor, e.g. `x86_64-apple-darwin` on an Apple-silicon Mac.

use std::path::PathBuf;

use kova_core::CoreKind;
use kova_engine::cores;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: bundle_cores <folder> [target-triple]"));
    let target = args.next().unwrap_or_default();
    let arch = if target.starts_with("aarch64") {
        "arm64"
    } else if target.starts_with("x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    std::fs::create_dir_all(&dir).expect("create the folder");
    for core in [CoreKind::Xray, CoreKind::Mihomo] {
        let tag = cores::install_latest_for(&dir, core, arch).await.unwrap_or_else(|e| panic!("{}: {e}", core.label()));
        println!("{} {tag} ({arch}), checksum ok", core.label());
    }
    // Only what Kova uses: the cores and the routing data.
    for entry in std::fs::read_dir(&dir).expect("read the folder").flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let keep = name.starts_with("mihomo") || name.starts_with("xray") || name.ends_with(".dat");
        if !keep || name.ends_with(".old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
