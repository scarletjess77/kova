//! Put verified Mihomo and Xray (and the routing data that comes with Xray) into a
//! folder, for the desktop installers to carry: Kova then starts without downloading
//! anything from GitHub, which is blocked in some countries.
//!
//! Usage: cargo run -p kova-engine --example bundle_cores -- <folder> [target-triple]
//! The target picks the processor, e.g. `x86_64-apple-darwin` on an Apple-silicon Mac.
//! `universal-apple-darwin` joins both Mac builds of each core into one file (needs lipo).

use std::path::{Path, PathBuf};

use kova_core::CoreKind;
use kova_engine::cores;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: bundle_cores <folder> [target-triple]"));
    let target = args.next().unwrap_or_default();
    if target == "universal-apple-darwin" {
        // Each processor's cores in a folder of their own, then joined file by file.
        let parts = ["arm64", "amd64"].map(|arch| (arch, std::env::temp_dir().join(format!("kova-cores-{arch}"))));
        for (arch, part) in &parts {
            let _ = std::fs::remove_dir_all(part);
            bundle(part, arch).await;
        }
        std::fs::create_dir_all(&dir).expect("create the folder");
        for entry in std::fs::read_dir(&parts[0].1).expect("read the folder").flatten() {
            let name = entry.file_name();
            let out = dir.join(&name);
            if name.to_string_lossy().ends_with(".dat") {
                // The routing data is the same for both.
                std::fs::copy(entry.path(), &out).expect("copy the routing data");
                continue;
            }
            let joined = std::process::Command::new("lipo").arg("-create").arg(entry.path()).arg(parts[1].1.join(&name)).arg("-output").arg(&out).status();
            assert!(joined.is_ok_and(|s| s.success()), "lipo could not join the two builds of {}", name.to_string_lossy());
        }
        for (_, part) in &parts {
            let _ = std::fs::remove_dir_all(part);
        }
        println!("joined the arm64 and amd64 builds");
        return;
    }
    let arch = if target.starts_with("aarch64") {
        "arm64"
    } else if target.starts_with("x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    bundle(&dir, arch).await;
}

/// The latest verified cores for `arch` and their routing data, in `dir`.
async fn bundle(dir: &Path, arch: &str) {
    std::fs::create_dir_all(dir).expect("create the folder");
    for core in [CoreKind::Xray, CoreKind::Mihomo] {
        let tag = cores::install_latest_for(dir, core, arch).await.unwrap_or_else(|e| panic!("{}: {e}", core.label()));
        println!("{} {tag} ({arch}), checksum ok", core.label());
    }
    // Only what Kova uses: the cores and the routing data.
    for entry in std::fs::read_dir(dir).expect("read the folder").flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let keep = name.starts_with("mihomo") || name.starts_with("xray") || name.ends_with(".dat");
        if !keep || name.ends_with(".old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
