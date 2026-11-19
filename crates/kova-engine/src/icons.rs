//! Program icons for the "Route an app" window: the icon of a program's .exe as a PNG data
//! URL, found by the name routing rules match. Windows only; elsewhere nothing is found.
//!
//! Results are kept in memory and as PNG files in the data folder, so a program that is not
//! running now still shows the icon it had when it was last seen.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use base64::Engine as _;

/// Icon files kept on disk; the oldest go first.
const MAX_FILES: usize = 300;
/// Longest program name looked up, in bytes: it becomes a file name.
const MAX_NAME: usize = 80;
/// Icons wider than this are not taken (Windows hands out 32 to 64 pixels).
const MAX_SIDE: u32 = 256;
/// A cached file larger than this is not an icon of ours.
const MAX_FILE: u64 = 256 * 1024;
/// Names and paths asked for at once.
const MAX_ASKED: usize = 600;
/// Entries held in memory before the map starts over.
const MAX_MEMORY: usize = 1500;

/// Lower-case name or path -> data URL.
static MEMORY: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

fn remembered(key: &str) -> Option<String> {
    MEMORY.lock().ok()?.get(key).cloned()
}

fn remember(key: &str, url: &str) {
    if let Ok(mut m) = MEMORY.lock() {
        if m.len() >= MAX_MEMORY {
            m.clear();
        }
        m.insert(key.to_string(), url.to_string());
    }
}

/// The lower-case form of a program name, or `None` when it cannot be a file name here
/// (a path, `..`, control characters, too long).
pub fn clean_name(name: &str) -> Option<String> {
    let n = name.trim();
    let bad = n.is_empty() || n.len() > MAX_NAME || n.contains("..") || n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'));
    (!bad).then(|| n.to_lowercase())
}

/// The file a cleaned name is kept in. Anything but letters, digits and `. - _ ( ) +` and
/// space becomes `%xx` per byte, so two names never share a file. A Windows device name
/// (`con`, `com3`, `lpt9`...) has its first byte escaped too (`%63on`): that is no device, and
/// no other name can produce it, because a `%` in a name is escaped itself. Non-ASCII
/// look-alikes of the digits (superscripts) are escaped per byte anyway.
pub fn cache_file_name(key: &str) -> String {
    let mut out = String::new();
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'(' | b')' | b'+' | b' ') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02x}"));
        }
    }
    // A trailing dot or space is dropped by Windows, which would merge two names.
    if out.ends_with(['.', ' ']) {
        let last = out.pop().unwrap_or_default();
        out.push_str(&format!("%{:02x}", last as u32));
    }
    // Windows treats `con .txt` like `con.txt`, so spaces before the first dot do not count.
    let stem = out.split('.').next().unwrap_or_default().trim_end_matches(' ');
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    if device {
        let first = out.remove(0);
        out.insert_str(0, &format!("%{:02x}", first as u32));
    }
    out.push_str(".png");
    out
}

/// Which of `files` (path, last written) to delete to keep `max`: the oldest.
pub fn prune_list(mut files: Vec<(PathBuf, SystemTime)>, max: usize) -> Vec<PathBuf> {
    if files.len() <= max {
        return Vec::new();
    }
    files.sort_by_key(|(_, t)| *t);
    let extra = files.len() - max;
    files.into_iter().take(extra).map(|(p, _)| p).collect()
}

fn prune(dir: &Path, max: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let files: Vec<(PathBuf, SystemTime)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .map(|e| (e.path(), e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)))
        .collect();
    for p in prune_list(files, max) {
        let _ = std::fs::remove_file(p);
    }
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// A PNG's bytes as a data URL.
pub fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

/// An RGBA picture of `w` x `h` pixels as a PNG; `None` when the size and the buffer disagree.
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || rgba.len() != (w * h * 4) as usize {
        return None;
    }
    // Every row starts with filter type 0 (none).
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for row in rgba.chunks_exact((w * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).ok()?;
    let idat = z.finish().ok()?;
    let mut head = Vec::with_capacity(13);
    head.extend_from_slice(&w.to_be_bytes());
    head.extend_from_slice(&h.to_be_bytes());
    head.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, b"IHDR", &head);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

fn read_cached(dir: &Path, key: &str) -> Option<String> {
    let path = dir.join(cache_file_name(key));
    if std::fs::metadata(&path).ok()?.len() > MAX_FILE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    bytes.starts_with(&SIGNATURE).then(|| data_url(&bytes))
}

/// Keeps `png` for `key` on disk; true when a file was written. Pruning is left to the caller,
/// so a batch scans the folder once.
fn write_cached(dir: &Path, key: &str, png: &[u8]) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let file = dir.join(cache_file_name(key));
    // Written beside and renamed, so a reader never meets half a file.
    let tmp = dir.join(format!("{}.tmp", cache_file_name(key)));
    if std::fs::write(&tmp, png).is_err() || std::fs::rename(&tmp, file).is_err() {
        // A failed write can leave a partial file behind; it is of no use to anyone.
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// Whether a path from the webview starts with a letter drive (`C:\` or `\\?\C:\`). UNC and
/// device paths are refused before the file system is touched: asking whether they exist could
/// reach out to the network or open a device.
pub fn is_drive_path(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
}

/// The icon of the program at `path`, as PNG bytes.
fn extract_png(path: &str) -> Option<Vec<u8>> {
    let (rgba, w, h) = imp::extract(path)?;
    encode_png(&rgba, w, h)
}

/// Icons for `names` (as running programs are called) and for the programs at `paths`
/// (chosen with the file dialog). The answer holds lower-case names, and only those an icon
/// was found for. A name is looked up in memory, then in `dir`, then in the running programs.
pub fn lookup(dir: &Path, names: &[String], paths: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut wrote = false;
    // A path was chosen on purpose: it wins over what is running under the same name.
    for p in paths.iter().take(MAX_ASKED) {
        let path = Path::new(p);
        let Some(key) = path.file_name().and_then(|n| clean_name(&n.to_string_lossy())) else { continue };
        let lower = p.to_lowercase();
        let url = remembered(&lower).or_else(|| {
            let ok = is_drive_path(path) && path.is_absolute() && lower.ends_with(".exe") && path.is_file();
            let png = if ok { extract_png(p)? } else { return None };
            let url = data_url(&png);
            remember(&lower, &url);
            wrote |= write_cached(dir, &key, &png);
            Some(url)
        });
        if let Some(url) = url {
            remember(&key, &url);
            out.insert(key, url);
        }
    }
    let mut running: Option<HashMap<String, String>> = None;
    for name in names.iter().take(MAX_ASKED) {
        let Some(key) = clean_name(name) else { continue };
        if out.contains_key(&key) {
            continue;
        }
        if let Some(url) = remembered(&key).or_else(|| read_cached(dir, &key)) {
            remember(&key, &url);
            out.insert(key, url);
            continue;
        }
        let at = running.get_or_insert_with(crate::programs::paths_by_name).get(&key).cloned();
        if let Some(png) = at.and_then(|p| extract_png(&p)) {
            let url = data_url(&png);
            remember(&key, &url);
            wrote |= write_cached(dir, &key, &png);
            out.insert(key, url);
        }
    }
    if wrote {
        prune(dir, MAX_FILES);
    }
    out
}

#[cfg(windows)]
mod imp {
    use std::ptr::null_mut;

    use windows_sys::Win32::Graphics::Gdi::{BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HBITMAP, ReleaseDC};
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    /// The bits of `bmp` (`w` x `h`) as top-down BGRA rows.
    fn bits(hdc: windows_sys::Win32::Graphics::Gdi::HDC, bmp: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
        // SAFETY: a zeroed header is filled in below, and the buffer is as big as GetDIBits writes.
        unsafe {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let got = GetDIBits(hdc, bmp, 0, h as u32, buf.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
            (got == h).then_some(buf)
        }
    }

    /// RGBA pixels of an icon handle.
    fn pixels(icon: HICON) -> Option<(Vec<u8>, u32, u32)> {
        // SAFETY: the bitmaps GetIconInfo hands out are ours to delete, and are, on every path out.
        unsafe {
            let mut info: ICONINFO = std::mem::zeroed();
            if GetIconInfo(icon, &mut info) == 0 {
                return None;
            }
            let found = (|| {
                // An icon with no colour bitmap is monochrome: not worth showing.
                if info.hbmColor.is_null() {
                    return None;
                }
                let mut bm: BITMAP = std::mem::zeroed();
                if GetObjectW(info.hbmColor, std::mem::size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast()) == 0 {
                    return None;
                }
                let (w, h) = (bm.bmWidth, bm.bmHeight);
                if w <= 0 || h <= 0 || w > super::MAX_SIDE as i32 || h > super::MAX_SIDE as i32 {
                    return None;
                }
                let hdc = GetDC(null_mut());
                if hdc.is_null() {
                    return None;
                }
                let color = bits(hdc, info.hbmColor, w, h);
                let mask = if info.hbmMask.is_null() { None } else { bits(hdc, info.hbmMask, w, h) };
                ReleaseDC(null_mut(), hdc);
                let mut px = color?;
                // Old icons carry no alpha: their transparency is in the mask (black = drawn).
                let has_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
                for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    p.swap(0, 2);
                    if !has_alpha {
                        p[3] = if mask.as_ref().is_none_or(|m| m[i * 4] == 0) { 255 } else { 0 };
                    }
                }
                px.as_chunks::<4>().0.iter().any(|p| p[3] != 0).then_some((px, w as u32, h as u32))
            })();
            DeleteObject(info.hbmColor);
            DeleteObject(info.hbmMask);
            found
        }
    }

    /// The first icon of the program at `path`, large size; `None` when it has none.
    pub fn extract(path: &str) -> Option<(Vec<u8>, u32, u32)> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return None;
        }
        // SAFETY: `wide` ends with a zero; the icon handle is destroyed after use.
        unsafe {
            let mut large: HICON = null_mut();
            // Unlike the shell's file-info call this finds only icons the file really has.
            ExtractIconExW(wide.as_ptr(), 0, &mut large, null_mut(), 1);
            if large.is_null() {
                return None;
            }
            let found = pixels(large);
            DestroyIcon(large);
            found
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn extract(_path: &str) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn names_that_are_not_file_names_are_skipped() {
        assert_eq!(clean_name("  Chrome.EXE "), Some("chrome.exe".into()));
        for bad in ["", "   ", "a/b.exe", "a\\b.exe", "..", "x..y", "c:evil", "a\u{0}b", &"a".repeat(MAX_NAME + 1)] {
            assert_eq!(clean_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn cache_file_names_are_safe_and_distinct() {
        assert_eq!(cache_file_name("chrome.exe"), "chrome.exe.png");
        assert_eq!(cache_file_name("my app (x64).exe"), "my app (x64).exe.png");
        let (a, b) = (cache_file_name("a b"), cache_file_name("a_b"));
        assert_ne!(a, b);
        assert_eq!(cache_file_name("a*b?"), "a%2ab%3f.png");
        assert_eq!(cache_file_name("é"), "%c3%a9.png");
        assert_eq!(cache_file_name("con"), "%63on.png");
        assert_eq!(cache_file_name("nul.txt"), "%6eul.txt.png");
        assert_eq!(cache_file_name("x."), "x%2e.png");
        assert_eq!(cache_file_name("x "), "x%20.png");
        assert_eq!(cache_file_name("x. "), "x.%20.png");
        for n in ["a/b", "..\\x", "x:y", "*"] {
            let f = cache_file_name(n);
            assert!(!f.contains(['/', '\\', ':', '*', '?']), "{f}");
        }
        // The longest name still fits a file name.
        assert!(cache_file_name(&"é".repeat(MAX_NAME / 2)).len() <= 255);
    }

    #[test]
    fn device_names_get_their_own_files() {
        let mut files = std::collections::HashSet::new();
        for raw in ["com3", "lpt9", "con", "_con", "con.exe", "COM3.EXE", "com0", "lpt0", "com1", "aux", "prn", "nul", "con .exe", "com10", "com", "conin$"] {
            let name = clean_name(raw).unwrap();
            let f = cache_file_name(&name);
            assert!(files.insert(f.clone()), "{raw} shares {f}");
            assert!(!f.starts_with("con.") && !f.starts_with("com3.") && !f.starts_with("lpt9."), "{f}");
        }
        assert_eq!(cache_file_name("com3"), "%63om3.png");
        assert_eq!(cache_file_name("lpt9"), "%6cpt9.png");
        assert_eq!(cache_file_name("com3.exe"), "%63om3.exe.png");
        assert_eq!(cache_file_name("_con"), "_con.png");
        assert_eq!(cache_file_name("com10"), "com10.png");
        assert_eq!(cache_file_name("conin$"), "conin%24.png");
    }

    #[cfg(windows)]
    #[test]
    fn only_letter_drives_pass_the_path_guard() {
        for ok in [r"C:\Windows\System32\notepad.exe", r"\\?\C:\Windows\x.exe", r"d:\x.exe"] {
            assert!(is_drive_path(Path::new(ok)), "{ok}");
        }
        for bad in [r"\\host\share\x.exe", "//host/share/x.exe", r"\\.\pipe\x.exe", r"\\?\GLOBALROOT\Device\x.exe", r"\\?\UNC\host\share\x.exe", r"x.exe", r"\x.exe", "/x.exe"] {
            assert!(!is_drive_path(Path::new(bad)), "{bad}");
        }
    }

    #[test]
    fn pruning_drops_the_oldest() {
        let t = |s| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let files = vec![("c".into(), t(30)), ("a".into(), t(10)), ("d".into(), t(40)), ("b".into(), t(20))];
        assert_eq!(prune_list(files.clone(), 2), vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert!(prune_list(files.clone(), 4).is_empty());
        assert_eq!(prune_list(files, 0).len(), 4);
    }

    #[test]
    fn prune_keeps_the_newest_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("{i}.png")), b"x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        std::fs::write(dir.path().join("note.txt"), b"x").unwrap();
        prune(dir.path(), 2);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["3.png", "4.png", "note.txt"]);
    }

    /// Reads a PNG made by `encode_png` back: header, size and pixels.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!(&png[24..29], &[8, 6, 0, 0, 0]);
        let len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&png[41..41 + len]).read_to_end(&mut raw).unwrap();
        assert_eq!(raw.len(), (h * (w * 4 + 1)) as usize);
        let pixels = raw.chunks_exact((w * 4 + 1) as usize).flat_map(|r| {
            assert_eq!(r[0], 0);
            r[1..].to_vec()
        });
        assert_eq!(&png[png.len() - 12..], &[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]);
        (w, h, pixels.collect())
    }

    #[test]
    fn rgba_becomes_a_png_data_url() {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8 * 7).collect();
        let png = encode_png(&rgba, 3, 2).unwrap();
        assert_eq!(decode(&png), (3, 2, rgba));
        let url = data_url(&png);
        let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(b64).unwrap(), png);
    }

    #[test]
    fn a_wrong_sized_buffer_gives_no_png() {
        assert!(encode_png(&[0; 15], 2, 2).is_none());
        assert!(encode_png(&[], 0, 0).is_none());
        assert!(encode_png(&vec![0; (MAX_SIDE as usize + 1) * 4], MAX_SIDE + 1, 1).is_none());
    }

    #[test]
    fn the_disk_cache_answers_for_programs_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let png = encode_png(&[255; 16], 2, 2).unwrap();
        write_cached(dir.path(), "kova-test-gone.exe", &png);
        assert!(dir.path().join("kova-test-gone.exe.png").is_file());
        let got = lookup(dir.path(), &["Kova-Test-Gone.exe".into(), "../evil.exe".into(), "kova-test-never.exe".into()], &[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got["kova-test-gone.exe"], data_url(&png));
        // Something that is not a PNG is no icon.
        std::fs::write(dir.path().join("kova-test-junk.exe.png"), b"hello").unwrap();
        assert_eq!(read_cached(dir.path(), "kova-test-junk.exe"), None);
    }

    /// Extracts a real icon through Win32 and checks the PNG that comes back.
    #[cfg(windows)]
    #[test]
    fn extracts_the_icon_of_a_system_program() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let found = [r"System32\notepad.exe", "explorer.exe"].iter().find_map(|p| extract_png(&format!(r"{root}\{p}")));
        let png = found.expect("an icon from notepad.exe or explorer.exe");
        let (w, h, px) = decode(&png);
        assert!((16..=MAX_SIDE).contains(&w) && w == h, "{w}x{h}");
        assert!(px.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
        // A program without an icon or a missing file is just no icon.
        assert!(extract_png(&format!(r"{root}\System32\no-such-file.exe")).is_none());
        assert!(extract_png("").is_none());
    }
}
