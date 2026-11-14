//! Kova's own log on disk, and the last crash, so a problem that closed Kova can still
//! be reported: `<data>/logs/kova.log` (the previous one is kept as `kova.log.1`) and
//! `<data>/logs/crash.log`. Logs in the window live only while it runs.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<Log>> = OnceLock::new();
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Above this the log starts over, keeping the old one beside it.
const MAX: u64 = 2 * 1024 * 1024;

/// The open log and how much it holds.
struct Log {
    file: fs::File,
    path: PathBuf,
    size: u64,
}

impl Log {
    /// The log as `kova.log.1` (replacing the one before it), and a new, empty log.
    /// Kova can run for weeks: started over only at launch, the log would grow all
    /// that time. Where the old one can't be moved, this one goes on.
    fn start_over(&mut self) {
        self.size = 0;
        let old = old_log(&self.path);
        let _ = fs::remove_file(&old);
        if fs::rename(&self.path, &old).is_ok()
            && let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(&self.path)
        {
            self.file = file;
        }
    }
}

/// Where the log before this one is kept: `kova.log.1` beside `kova.log`.
fn old_log(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    name.into()
}

/// A `tracing` writer: the log file once [`start`] has opened it, nothing before.
pub struct Writer;

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match FILE.get().and_then(|f| f.lock().ok()) {
            Some(mut log) => {
                if log.size > MAX {
                    log.start_over();
                }
                let n = log.file.write(buf)?;
                log.size += n as u64;
                Ok(n)
            }
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match FILE.get().and_then(|f| f.lock().ok()) {
            Some(mut log) => log.file.flush(),
            None => Ok(()),
        }
    }
}

/// Open the log in `data`'s logs folder and record panics there from now on.
pub fn start(data: &Path) {
    let dir = data.join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("kova.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX) {
        let _ = fs::rename(&path, old_log(&path));
    }
    if let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let size = file.metadata().map_or(0, |m| m.len());
        let _ = FILE.set(Mutex::new(Log { file, path, size }));
    }
    let _ = DIR.set(dir);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = format!(
            "Kova {} crashed on {} {}\n{info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            utc(now()),
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(dir) = DIR.get() {
            let _ = fs::write(dir.join("crash.log"), &text);
        }
        tracing::error!("{info}");
        previous(info);
    }));
}

/// The last crash, if there was one: when (Unix seconds) and what was recorded.
pub fn last_crash() -> Option<(i64, String)> {
    let path = DIR.get()?.join("crash.log");
    let when = fs::metadata(&path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((when, fs::read_to_string(path).ok()?))
}

/// Whether a crash happened since the last time this was asked, so it is mentioned once.
pub fn crash_is_new() -> bool {
    let (Some(dir), Some((when, _))) = (DIR.get(), last_crash()) else { return false };
    let seen = dir.join("crash.seen");
    let before = fs::read_to_string(&seen).ok().and_then(|s| s.trim().parse::<i64>().ok());
    let _ = fs::write(&seen, when.to_string());
    before != Some(when)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `secs` as "2026-10-28 14:05 UTC", without a date library.
fn utc(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02} {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_full_log_starts_over_and_keeps_the_one_before() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kova.log");
        std::fs::write(&path, "old lines
").unwrap();
        let file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let mut log = super::Log { file, path: path.clone(), size: super::MAX + 1 };
        log.start_over();
        log.file.write_all(b"new line
").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new line
");
        assert_eq!(std::fs::read_to_string(dir.path().join("kova.log.1")).unwrap(), "old lines
");
        assert_eq!(log.size, 0);
    }

    #[test]
    fn dates_are_right() {
        assert_eq!(super::utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(super::utc(1_793_160_000), "2026-10-28 04:00 UTC");
        assert_eq!(super::utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}
