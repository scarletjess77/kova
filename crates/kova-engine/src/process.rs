//! Running a core as a child process and streaming its output.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::Error;

/// Hide the console window a console-subsystem child would otherwise open on Windows.
pub fn hide_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Whether Kova runs with the rights TUN mode needs (an elevated token on Windows,
/// root elsewhere).
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
    #[cfg(unix)]
    {
        // Root. On Linux TUN can also run without it, through the copy of Mihomo with
        // network rights (see `tun_linux`).
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(any(windows, unix)))]
    {
        true
    }
}

/// A line of core output.
pub type LineSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub struct CoreProcess {
    pub name: &'static str,
    child: Child,
    tail: Arc<Mutex<Vec<String>>>,
    /// `<name>.pid` in the core's folder, so a Kova that crashed can be followed by one
    /// that ends its cores (see [`end_leftovers`]).
    pid_file: std::path::PathBuf,
}

impl CoreProcess {
    /// Start `bin` with `args`. Every output line goes to `sink(name, line)`; the last
    /// lines are also kept so a failed start can be explained.
    pub fn spawn(name: &'static str, bin: &Path, args: &[&str], cwd: &Path, envs: &[(&str, &Path)], sink: LineSink) -> Result<Self, Error> {
        Self::spawn_with_fd(name, bin, args, cwd, envs, sink, None)
    }

    /// Like `spawn`, handing the child the open descriptor `fd` (Android's VPN
    /// interface). It is made inheritable in the child only, just before the core
    /// starts, so no other process Kova starts ever holds it.
    pub fn spawn_with_fd(name: &'static str, bin: &Path, args: &[&str], cwd: &Path, envs: &[(&str, &Path)], sink: LineSink, fd: Option<i32>) -> Result<Self, Error> {
        if !bin.exists() {
            return Err(Error::Core(format!("{name} is not installed")));
        }
        let mut std_cmd = std::process::Command::new(bin);
        std_cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in envs {
            std_cmd.env(k, v);
        }
        // Phones and TV boxes are short of memory: past this the Go cores collect garbage
        // sooner instead of growing. A soft limit, so a busy core can still go over it.
        #[cfg(target_os = "android")]
        std_cmd.env("GOMEMLIMIT", "96MiB");
        hide_window(&mut std_cmd);
        #[cfg(unix)]
        if let Some(fd) = fd {
            use std::os::unix::process::CommandExt;
            // Runs in the child between fork and exec.
            unsafe {
                std_cmd.pre_exec(move || {
                    if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(not(unix))]
        let _ = fd;
        let mut cmd = Command::from(std_cmd);
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| Error::Core(format!("could not start {name}: {e}")))?;
        job::adopt(&child);
        let pid_file = cwd.join(format!("{name}.pid"));
        if let Some(pid) = child.id() {
            let _ = std::fs::write(&pid_file, pid.to_string());
        }

        let tail = Arc::new(Mutex::new(Vec::new()));
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|s| Box::new(s) as _)].into_iter().flatten() {
            let sink = sink.clone();
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    {
                        let mut t = tail.lock().unwrap();
                        t.push(line.clone());
                        if t.len() > 40 {
                            t.remove(0);
                        }
                    }
                    sink(name, &line);
                }
            });
        }
        Ok(Self { name, child, tail, pid_file })
    }

    /// `Some(message)` if the process has already exited.
    pub fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("{} stopped ({status}). {}", self.name, self.last_error())),
            _ => None,
        }
    }

    /// The most informative recent line, for error messages.
    pub fn last_error(&self) -> String {
        let t = self.tail.lock().unwrap();
        t.iter()
            .rev()
            .find(|l| {
                let l = l.to_ascii_lowercase();
                l.contains("fatal") || l.contains("error") || l.contains("failed") || l.contains("panic")
            })
            .or_else(|| t.last())
            .cloned()
            .unwrap_or_default()
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
        let _ = std::fs::remove_file(&self.pid_file);
    }
}

/// End cores a previous Kova left running: on Windows they die with Kova (see `job`), but
/// elsewhere a crash leaves them holding the proxy port, and the next connect would fail.
/// Only a process still running one of `bins` is ended, so a reused process number is
/// left alone. Looks for `<core>.pid` files in `dir` and the folders in it.
pub fn end_leftovers(dir: &Path, bins: &[std::path::PathBuf]) {
    let bins: Vec<std::path::PathBuf> = bins.iter().map(|b| std::fs::canonicalize(b).unwrap_or_else(|_| b.clone())).collect();
    let mut dirs = vec![dir.to_path_buf()];
    let mut i = 0;
    while i < dirs.len() && i < 32 {
        if let Ok(entries) = std::fs::read_dir(&dirs[i]) {
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    if dirs.len() < 32 {
                        dirs.push(path);
                    }
                } else if path.extension().is_some_and(|x| x == "pid") {
                    #[cfg(unix)]
                    if let Some(pid) = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<i32>().ok()) {
                        if pid > 1 && image_of(pid).is_some_and(|img| bins.contains(&img)) {
                            // SAFETY: plain signal to a process checked to be one of our cores.
                            unsafe { libc::kill(pid, libc::SIGKILL) };
                            tracing::info!("ended a core left running by a previous Kova (process {pid})");
                        }
                    }
                    #[cfg(not(unix))]
                    let _ = bins;
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        i += 1;
    }
}

/// The program process `pid` runs.
#[cfg(unix)]
fn image_of(pid: i32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!path.is_empty()).then(|| std::path::PathBuf::from(path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        // A binary replaced by an update since reads "<path> (deleted)".
        let link = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        let text = link.to_string_lossy();
        Some(std::path::PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text)))
    }
}

/// Run a core once to validate a config file. `Ok(())` or the core's complaint.
pub async fn check_config(bin: &Path, args: &[&str], cwd: &Path, envs: &[(&str, &Path)]) -> Result<(), Error> {
    let mut c = std::process::Command::new(bin);
    c.args(args).current_dir(cwd);
    for (k, v) in envs {
        c.env(k, v);
    }
    hide_window(&mut c);
    let out = Command::from(c).output().await.map_err(|e| Error::Core(e.to_string()))?;
    if out.status.success() {
        return Ok(());
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let msg = text
        .lines()
        .rev()
        .find(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("error") || l.contains("fatal") || l.contains("failed")
        })
        .unwrap_or(text.lines().last().unwrap_or("unknown error"));
    Err(Error::Config(msg.trim().to_string()))
}

#[cfg(windows)]
mod job {
    //! All cores join one Job Object that kills its members when Kova's last handle
    //! closes, i.e. when Kova exits for any reason, including a crash.
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    struct Job(HANDLE);
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(h);
                return None;
            }
            Some(Job(h))
        })
        .as_ref()
    }

    pub fn adopt(child: &tokio::process::Child) {
        let (Some(job), Some(pid)) = (job(), child.id()) else { return };
        unsafe {
            let p = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !p.is_null() {
                AssignProcessToJobObject(job.0, p);
                CloseHandle(p);
            }
        }
    }
}

#[cfg(not(windows))]
mod job {
    pub fn adopt(_child: &tokio::process::Child) {}
}

#[cfg(test)]
mod tests {
    /// A core a crashed Kova left behind is ended; its pid file goes either way.
    #[test]
    fn leftover_cores_are_ended() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("main");
        std::fs::create_dir_all(&run).unwrap();
        #[cfg(target_os = "linux")]
        {
            let bin = dir.path().join("mihomo");
            std::fs::copy("/bin/sleep", &bin).unwrap();
            let mut child = std::process::Command::new(&bin).arg("30").spawn().unwrap();
            std::fs::write(run.join("mihomo.pid"), child.id().to_string()).unwrap();
            // Another program under the same number is left alone.
            let mut other = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
            std::fs::write(run.join("xray.pid"), other.id().to_string()).unwrap();
            super::end_leftovers(dir.path(), &[bin]);
            assert!(child.wait().unwrap().code().is_none(), "ended by a signal");
            assert!(other.try_wait().unwrap().is_none(), "not one of ours");
            let _ = other.kill();
        }
        std::fs::write(run.join("mihomo.pid"), "1").unwrap();
        super::end_leftovers(dir.path(), &[]);
        assert!(!run.join("mihomo.pid").exists());
    }
}
