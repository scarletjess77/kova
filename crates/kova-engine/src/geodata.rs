//! Downloading and installing GeoIP / GeoSite databases.
//!
//! The files live in the cores folder (Xray reads them from there through
//! `XRAY_LOCATION_ASSET`) and are hard-linked into each Mihomo home as `GeoIP.dat` /
//! `GeoSite.dat`, so the ~90 MB Russian lists exist once no matter how many homes use
//! them. `geodata.json` beside them records where they came from.
//!
//! Every source also keeps its own copy in `geodata-cache/<id>/`, so switching back to a
//! source downloaded recently is a local copy, not another 90 MB download.

use std::collections::HashMap;
use std::path::Path;

use kova_core::options::GeoSource;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::{now, write_atomic};
use crate::{Error, fetch};

pub const FILES: [&str; 2] = ["geoip.dat", "geosite.dat"];
const MARKER: &str = "geodata.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Marker {
    pub source: String,
    /// Checked against the publisher's SHA-256 digest.
    pub verified: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub categories: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeoInfo {
    /// What the installed files came from; `None` when they shipped with Xray.
    pub installed: Option<Marker>,
    pub files: Vec<FileInfo>,
    /// Categories the enabled presets need but the files don't have, e.g. `geosite:ru-blocked`.
    pub missing: Vec<String>,
}

pub fn marker(cores_dir: &Path) -> Option<Marker> {
    std::fs::read(cores_dir.join(MARKER)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Category names of an installed file, or `None` if it is absent or unreadable.
/// Parsing the Russian lists means reading ~90 MB, so results are kept until the file
/// changes (by size or modification time).
fn categories_of(path: &Path) -> Option<std::sync::Arc<Vec<String>>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Key = (u64, Option<std::time::SystemTime>);
    type Cache = Mutex<HashMap<std::path::PathBuf, (Key, Arc<Vec<String>>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    let key = (meta.len(), meta.modified().ok());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((k, cats)) = cache.lock().unwrap().get(path)
        && *k == key {
            return Some(cats.clone());
        }
    let data = std::fs::read(path).ok()?;
    let cats = Arc::new(kova_core::geodata::categories(&data).ok()?);
    cache.lock().unwrap().insert(path.to_path_buf(), (key, cats.clone()));
    Some(cats)
}

/// Describe the installed files and check them against the categories presets need.
pub fn info(cores_dir: &Path, needed_site: &[&str], needed_ip: &[&str]) -> GeoInfo {
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for (name, needed, prefix) in [("geoip.dat", needed_ip, "geoip"), ("geosite.dat", needed_site, "geosite")] {
        let path = cores_dir.join(name);
        let Some(cats) = categories_of(&path) else { continue };
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        for n in needed {
            if !cats.iter().any(|c| c == n) {
                missing.push(format!("{prefix}:{n}"));
            }
        }
        files.push(FileInfo { name: name.to_string(), size, categories: cats.len() });
    }
    GeoInfo { installed: marker(cores_dir), files, missing }
}

/// How long a downloaded copy counts as current.
pub fn max_age(source: &GeoSource) -> i64 {
    match source {
        GeoSource::Runetfreedom => 12 * 3600,
        _ => 7 * 86400,
    }
}

fn cache_dir(cores_dir: &Path, source: &GeoSource) -> std::path::PathBuf {
    let id = hex::encode(&Sha256::digest(source.id().as_bytes())[..6]);
    cores_dir.join("geodata-cache").join(id)
}

fn is_fresh(dir: &Path, source: &GeoSource) -> Option<Marker> {
    let m = marker(dir)?;
    let complete = FILES.iter().all(|f| dir.join(f).exists());
    (complete && m.source == source.id() && now() - m.updated_at < max_age(source)).then_some(m)
}

/// Put `from`'s bytes at `to`, keeping the modification time, so later "is it the same
/// file" checks work. A hard link where the filesystem allows one — the linked places
/// then share one copy of the file — and a plain copy where it does not.
fn place(from: &Path, to: &Path) -> std::io::Result<()> {
    let tmp = crate::store::temp_path(to);
    if let Err(err) = std::fs::hard_link(from, &tmp) {
        tracing::warn!("{}: could not link it ({err}); copying it instead", to.display());
        std::fs::copy(from, &tmp)?;
        if let Ok(t) = std::fs::metadata(from).and_then(|m| m.modified()) {
            let _ = std::fs::File::options().write(true).open(&tmp).and_then(|f| f.set_modified(t));
        }
    }
    std::fs::rename(&tmp, to).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Whether both paths are the same file on disk (a hard link), where that can be told.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

/// Copies of the active files that earlier versions left in the download cache are
/// the same bytes: link them to the active files so each list is on disk once.
pub fn share_cached_copies(cores_dir: &Path) {
    let Ok(caches) = std::fs::read_dir(cores_dir.join("geodata-cache")) else { return };
    let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
    for cache in caches.flatten() {
        for f in FILES {
            let (active, copy) = (cores_dir.join(f), cache.path().join(f));
            if stamp(&active).is_some() && stamp(&active) == stamp(&copy) && !same_file(&active, &copy) {
                let _ = place(&active, &copy);
            }
        }
    }
}

/// The big files under `root`, with how many names each has, and how much disk they
/// use once a file with several names is counted once; one line each, for the log.
pub fn usage_report(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<(std::path::PathBuf, std::fs::Metadata)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                walk(&e.path(), out);
            } else if m.len() >= 5 << 20 {
                out.push((e.path(), m));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut seen = std::collections::HashSet::new();
    let (mut total, mut once) = (0u64, 0u64);
    let mut lines = Vec::new();
    for (path, m) in &files {
        #[cfg(unix)]
        let (names, id) = {
            use std::os::unix::fs::MetadataExt;
            (m.nlink(), Some((m.dev(), m.ino())))
        };
        #[cfg(not(unix))]
        let (names, id) = (1, None::<(u64, u64)>);
        total += m.len();
        if id.is_none_or(|id| seen.insert(id)) {
            once += m.len();
        }
        let shown = path.strip_prefix(root).unwrap_or(path);
        lines.push(format!("storage: {} {} MB, {names} name(s)", shown.display(), m.len() >> 20));
    }
    lines.push(format!("storage: big files {} MB by name, {} MB on disk", total >> 20, once >> 20));
    lines
}

/// Make the copy in `from` the active one.
fn install(from: &Path, cores_dir: &Path) -> Result<Marker, Error> {
    for f in FILES.iter().chain(std::iter::once(&MARKER)) {
        place(&from.join(f), &cores_dir.join(f))?;
    }
    marker(cores_dir).ok_or_else(|| Error::Other("the routing data could not be installed".into()))
}

/// Download `source` again, verify it and make it active.
pub async fn update(cores_dir: &Path, source: &GeoSource, proxy: Option<&str>) -> Result<Marker, Error> {
    let cache = cache_dir(cores_dir, source);
    download_into(&cache, source, proxy).await?;
    install(&cache, cores_dir)
}

/// Make `source` active, downloading only if there is no current copy of it. Returns
/// the marker and whether a download happened.
pub async fn switch_to(cores_dir: &Path, source: &GeoSource, proxy: Option<&str>) -> Result<(Marker, bool), Error> {
    if let Some(m) = is_fresh(cores_dir, source) {
        return Ok((m, false));
    }
    let cache = cache_dir(cores_dir, source);
    if is_fresh(&cache, source).is_some() {
        return Ok((install(&cache, cores_dir)?, false));
    }
    Ok((update(cores_dir, source, proxy).await?, true))
}

/// Make sure `source`'s data is installed, downloading only when it is missing or came
/// from another source. Unlike [`switch_to`] an old copy is kept: used when a download
/// would hold up connecting, while regular refreshes happen elsewhere.
pub async fn ensure(cores_dir: &Path, source: &GeoSource, proxy: Option<&str>) -> Result<(Marker, bool), Error> {
    if let Some(m) = marker(cores_dir)
        && m.source == source.id() && FILES.iter().all(|f| cores_dir.join(f).exists()) {
            return Ok((m, false));
        }
    switch_to(cores_dir, source, proxy).await
}

/// Download both files from `source` into `dir`, verifying and validating them. Nothing
/// is replaced unless both files check out.
async fn download_into(dir: &Path, source: &GeoSource, proxy: Option<&str>) -> Result<Marker, Error> {
    let c = fetch::client(proxy, fetch::APP_UA)?;
    let digests: HashMap<String, String> = match source.release_api() {
        Some(api) => {
            let r = c.get(api).header("Accept", "application/vnd.github+json").send().await?.error_for_status()?;
            let v: serde_json::Value = r.json().await?;
            v["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["digest"].as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase())))
                .collect()
        }
        None => HashMap::new(),
    };
    let (geoip_url, geosite_url) = source.urls();
    if geoip_url.is_empty() || geosite_url.is_empty() {
        return Err(Error::Other("Enter both download addresses for the custom routing data".into()));
    }
    // Big files (the Russian lists are ~70 MB): no overall timeout, only a connect timeout.
    let big = fetch::builder().user_agent(fetch::APP_UA).connect_timeout(std::time::Duration::from_secs(15));
    let big = match proxy {
        Some(p) => big.proxy(reqwest::Proxy::all(p)?),
        None => big.no_proxy(),
    }
    .build()?;
    let mut downloaded = Vec::new();
    for (name, url) in [("geoip.dat", geoip_url), ("geosite.dat", geosite_url)] {
        let data = big.get(&url).send().await?.error_for_status()?.bytes().await?;
        if let Some(expected) = digests.get(name) {
            let actual = hex::encode(Sha256::digest(&data));
            if &actual != expected {
                return Err(Error::Other(format!("{name} failed its checksum check (expected {expected}, got {actual})")));
            }
        } else if source.release_api().is_some() {
            return Err(Error::Other(format!("the release has no checksum for {name}; refusing to install it")));
        }
        let cats = kova_core::geodata::categories(&data).map_err(|e| Error::Other(format!("{name} from {url}: {e}")))?;
        tracing::info!("{name}: {} categories, {} bytes", cats.len(), data.len());
        downloaded.push((name, data));
    }
    std::fs::create_dir_all(dir)?;
    for (name, data) in downloaded {
        write_atomic(&dir.join(name), &data)?;
    }
    let m = Marker { source: source.id(), verified: source.release_api().is_some(), updated_at: now() };
    write_atomic(&dir.join(MARKER), &serde_json::to_vec_pretty(&m).expect("json"))?;
    Ok(m)
}

/// Link the installed files into a Mihomo home when its copies differ. "Differ", not
/// "older": switching back to a cached source installs files with an earlier date.
pub fn seed_mihomo_home(cores_dir: &Path, home: &Path) {
    for (src, dst) in [("geoip.dat", "GeoIP.dat"), ("geosite.dat", "GeoSite.dat")] {
        let from = cores_dir.join(src);
        let to = home.join(dst);
        let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
        match stamp(&from) {
            Some(f) if stamp(&to) != Some(f) || !same_file(&from, &to) => {
                let _ = place(&from, &to);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_install(dir: &Path, source: &GeoSource, age_secs: i64) {
        // A minimal valid geodata file: one entry named "private".
        let file = [0x0a, 0x09, 0x0a, 0x07, b'p', b'r', b'i', b'v', b'a', b't', b'e'];
        for f in FILES {
            std::fs::write(dir.join(f), file).unwrap();
        }
        let m = Marker { source: source.id(), verified: true, updated_at: now() - age_secs };
        std::fs::write(dir.join(MARKER), serde_json::to_vec(&m).unwrap()).unwrap();
    }

    #[tokio::test]
    async fn ensure_keeps_an_old_copy_of_the_same_source() {
        let dir = tempfile::tempdir().unwrap();
        // Older than its refresh interval, but the right source: no download (this would
        // fail without network if it tried).
        fake_install(dir.path(), &GeoSource::Runetfreedom, 30 * 86400);
        let (m, downloaded) = ensure(dir.path(), &GeoSource::Runetfreedom, None).await.unwrap();
        assert!(!downloaded);
        assert_eq!(m.source, "runetfreedom");
        assert_eq!(info(dir.path(), &["private"], &[]).missing, Vec::<String>::new());
    }

    #[test]
    fn temp_names_never_collide() {
        let p = Path::new("x/geosite.dat");
        let a = crate::store::temp_path(p);
        let b = crate::store::temp_path(p);
        assert_ne!(a, b);
        assert_eq!(a.parent(), p.parent());
    }

    #[test]
    fn homes_share_one_copy_where_the_filesystem_allows() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("geoip.dat"), [1u8, 2, 3]).unwrap();
        place(&a.join("geoip.dat"), &b.join("geoip.dat")).unwrap();
        assert_eq!(std::fs::read(b.join("geoip.dat")).unwrap(), [1, 2, 3]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert!(same_file(&a.join("geoip.dat"), &b.join("geoip.dat")));
            let (ma, mb) = (
                std::fs::metadata(a.join("geoip.dat")).unwrap(),
                std::fs::metadata(b.join("geoip.dat")).unwrap(),
            );
            assert_eq!((ma.dev(), ma.ino()), (mb.dev(), mb.ino()));
        }
    }

    #[test]
    fn old_separate_copies_are_linked_again() {
        let dir = tempfile::tempdir().unwrap();
        let cores = dir.path().join("cores");
        let cache = cores.join("geodata-cache").join("x");
        let home = dir.path().join("home");
        for d in [&cache, &home] {
            std::fs::create_dir_all(d).unwrap();
        }
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        for (dir, name) in [(&cores, "geoip.dat"), (&cache, "geoip.dat"), (&home, "GeoIP.dat")] {
            let f = std::fs::File::create(dir.join(name)).unwrap();
            std::io::Write::write_all(&mut &f, &[7u8; 64]).unwrap();
            f.set_modified(t).unwrap();
        }
        share_cached_copies(&cores);
        seed_mihomo_home(&cores, &home);
        assert_eq!(std::fs::read(home.join("GeoIP.dat")).unwrap(), [7u8; 64]);
        #[cfg(unix)]
        {
            assert!(same_file(&cores.join("geoip.dat"), &cache.join("geoip.dat")));
            assert!(same_file(&cores.join("geoip.dat"), &home.join("GeoIP.dat")));
        }
    }
}
