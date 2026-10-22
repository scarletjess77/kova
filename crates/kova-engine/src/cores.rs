//! Installing and updating the Mihomo and Xray binaries from their GitHub releases.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use kova_core::CoreKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, fetch};

pub fn binary_name(core: CoreKind) -> &'static str {
    match (core, cfg!(windows)) {
        (CoreKind::Mihomo, true) => "mihomo.exe",
        (CoreKind::Mihomo, false) => "mihomo",
        (CoreKind::Xray, true) => "xray.exe",
        (CoreKind::Xray, false) => "xray",
    }
}

pub fn binary_path(cores_dir: &Path, core: CoreKind) -> PathBuf {
    #[cfg(target_os = "android")]
    if let Some(dir) = android::bundled_dir() {
        return dir.join(android::lib_name(core));
    }
    cores_dir.join(binary_name(core))
}

/// Android runs only programs that ship inside the app, so there the cores come as
/// native libraries in the app's library folder, next to Kova's own, and are updated
/// with the app.
#[cfg(target_os = "android")]
mod android {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use kova_core::CoreKind;

    pub fn lib_name(core: CoreKind) -> &'static str {
        match core {
            CoreKind::Mihomo => "libmihomo.so",
            CoreKind::Xray => "libxray.so",
        }
    }

    /// The folder Android unpacked the app's libraries to, found from where Kova's own
    /// library is loaded.
    pub fn bundled_dir() -> Option<&'static Path> {
        static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
        DIR.get_or_init(|| {
            let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
            let lib = maps.lines().filter_map(|l| l.split_whitespace().nth(5)).find(|p| p.ends_with("/libkova_app.so"))?;
            Path::new(lib).parent().map(Path::to_path_buf)
        })
        .as_deref()
    }
}

fn repo(core: CoreKind) -> &'static str {
    match core {
        CoreKind::Mihomo => "MetaCubeX/mihomo",
        CoreKind::Xray => "XTLS/Xray-core",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreStatus {
    pub core: CoreKind,
    pub installed: Option<String>,
    pub latest: Option<String>,
}

impl CoreStatus {
    pub fn update_available(&self) -> bool {
        match (&self.installed, &self.latest) {
            (Some(i), Some(l)) => normalise(i) != normalise(l),
            (None, Some(_)) => true,
            _ => false,
        }
    }
}

fn normalise(v: &str) -> &str {
    v.trim_start_matches('v')
}

/// Version reported by the installed binary, e.g. `v1.19.15` or `25.10.15`.
pub fn installed_version(cores_dir: &Path, core: CoreKind) -> Option<String> {
    let bin = binary_path(cores_dir, core);
    if !bin.exists() {
        return None;
    }
    let arg = match core {
        CoreKind::Mihomo => "-v",
        CoreKind::Xray => "version",
    };
    let mut cmd = Command::new(bin);
    cmd.arg(arg);
    crate::process::hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    // "Mihomo Meta v1.19.15 windows amd64 with go1.25 ..." / "Xray 25.10.15 (Xray, Penetrates Everything.) ..."
    first.split_whitespace().find(|w| w.trim_start_matches('v').chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.')).map(String::from)
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

async fn latest_release(core: CoreKind) -> Result<Release, Error> {
    let c = fetch::client(None, fetch::APP_UA)?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo(core));
    let r = c.get(url).header("Accept", "application/vnd.github+json").send().await?;
    if !r.status().is_success() {
        return Err(Error::Http(format!("GitHub answered {}", r.status())));
    }
    Ok(r.json().await?)
}

pub async fn latest_version(core: CoreKind) -> Result<String, Error> {
    Ok(latest_release(core).await?.tag_name)
}

fn os_arch() -> (&'static str, &'static str) {
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
    (os, arch)
}

/// Pick the release asset for this machine, most compatible build first.
fn pick_asset<'a>(core: CoreKind, rel: &'a Release) -> Option<&'a Asset> {
    let (os, arch) = os_arch();
    let tag = &rel.tag_name;
    let candidates: Vec<String> = match core {
        CoreKind::Mihomo => {
            let ext = if os == "windows" { "zip" } else { "gz" };
            let mut c = Vec::new();
            if arch == "amd64" {
                // The default amd64 build needs AVX2-era CPUs; the compatible one runs anywhere.
                c.push(format!("mihomo-{os}-{arch}-compatible-{tag}.{ext}"));
                c.push(format!("mihomo-{os}-{arch}-v1-{tag}.{ext}"));
            }
            c.push(format!("mihomo-{os}-{arch}-{tag}.{ext}"));
            c
        }
        CoreKind::Xray => {
            let os = if os == "darwin" { "macos" } else { os };
            let a = if arch == "arm64" { "arm64-v8a" } else { "64" };
            vec![format!("Xray-{os}-{a}.zip")]
        }
    };
    candidates.iter().find_map(|c| rel.assets.iter().find(|a| &a.name == c))
}

/// Expected SHA-256 of an asset: GitHub's own digest, or Xray's `.dgst` side file.
async fn expected_sha256(c: &reqwest::Client, rel: &Release, asset: &Asset) -> Result<Option<String>, Error> {
    if let Some(d) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        return Ok(Some(d.to_ascii_lowercase()));
    }
    let side = format!("{}.dgst", asset.name);
    if let Some(dg) = rel.assets.iter().find(|a| a.name == side) {
        let text = c.get(&dg.browser_download_url).send().await?.text().await?;
        // Lines look like "SHA2-256= <hex>".
        let hex = text.lines().find(|l| l.starts_with("SHA2-256") || l.starts_with("SHA256")).and_then(|l| l.split('=').nth(1)).map(|h| h.trim().to_ascii_lowercase());
        return Ok(hex);
    }
    Ok(None)
}

/// Download, verify and install the latest release. Returns the installed tag.
pub async fn install_latest(cores_dir: &Path, core: CoreKind) -> Result<String, Error> {
    if cfg!(target_os = "android") {
        return Err(Error::Core(format!("On Android, {} is updated together with Kova", core.label())));
    }
    let rel = latest_release(core).await?;
    let asset = pick_asset(core, &rel).ok_or_else(|| Error::Core(format!("no {} build for this system in {}", core.label(), rel.tag_name)))?;
    let c = fetch::client(None, fetch::APP_UA)?;
    let expected = expected_sha256(&c, &rel, asset).await?.ok_or_else(|| Error::Core(format!("{} has no published checksum; refusing to install it", asset.name)))?;
    let data = c.get(&asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
    let actual = hex::encode(Sha256::digest(&data));
    if actual != expected {
        return Err(Error::Core(format!("checksum mismatch for {}: expected {expected}, got {actual}", asset.name)));
    }

    let staging = cores_dir.join(format!(".staging-{}", core.label().to_lowercase()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let bin_name = binary_name(core);
    let data = data.to_vec();
    let staged_bin = staging.join(bin_name);
    let staging2 = staging.clone();
    let asset_name = asset.name.clone();
    let keep_geodata = crate::geodata::marker(cores_dir).is_some();
    tokio::task::spawn_blocking(move || extract(&asset_name, &data, core, &staging2, keep_geodata)).await.expect("extract task")?;
    if !staged_bin.exists() {
        return Err(Error::Core(format!("{} did not contain {bin_name}", asset.name)));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged_bin, std::fs::Permissions::from_mode(0o755))?;
    }
    for entry in std::fs::read_dir(&staging)? {
        let entry = entry?;
        let dest = cores_dir.join(entry.file_name());
        // On Windows a running binary can't be replaced but can be renamed out of the way.
        if dest.exists() {
            let old = dest.with_extension("old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&dest, &old)?;
        }
        std::fs::rename(entry.path(), &dest)?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(rel.tag_name)
}

fn extract(name: &str, data: &[u8], core: CoreKind, out: &Path, keep_geodata: bool) -> Result<(), Error> {
    let bin = binary_name(core);
    if name.ends_with(".gz") {
        let mut d = flate2::read::GzDecoder::new(data);
        let mut buf = Vec::new();
        d.read_to_end(&mut buf)?;
        std::fs::write(out.join(bin), buf)?;
        return Ok(());
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| Error::Core(format!("bad archive: {e}")))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| Error::Core(format!("bad archive: {e}")))?;
        if f.is_dir() {
            continue;
        }
        let entry_name = f.name().map(|n| n.to_string()).unwrap_or_default();
        let file = Path::new(&entry_name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let lower = file.to_ascii_lowercase();
        // Mihomo's archive holds e.g. `mihomo-windows-amd64-compatible.exe`; Xray's holds `xray.exe`.
        let is_core = match core {
            CoreKind::Mihomo => lower.starts_with("mihomo") && (lower.ends_with(".exe") || !lower.contains('.')),
            CoreKind::Xray => lower == "xray.exe" || lower == "xray",
        };
        let target = if is_core {
            bin.to_string()
        } else if lower.ends_with(".dat") && !keep_geodata {
            // Xray ships default geodata; don't overwrite files from a source the user chose.
            file
        } else {
            continue;
        };
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        std::fs::write(out.join(target), buf)?;
    }
    Ok(())
}

pub async fn status(cores_dir: &Path, core: CoreKind, check_latest: bool) -> CoreStatus {
    let dir = cores_dir.to_path_buf();
    let installed = tokio::task::spawn_blocking(move || installed_version(&dir, core)).await.ok().flatten();
    // On Android the cores can't be replaced separately, so there is nothing to offer.
    let latest = if check_latest && !cfg!(target_os = "android") { latest_version(core).await.ok() } else { None };
    CoreStatus { core, installed, latest }
}
