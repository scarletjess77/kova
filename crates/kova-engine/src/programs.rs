//! The programs running on this computer, by the name routing rules match
//! (`PROCESS-NAME`): what Settings offers when choosing an app to route. Only the user's
//! own programs: system processes are left out, and so is Kova with its cores.

/// Names Kova and its cores run under; they are routed by Kova already.
const OWN: &[&str] = &["kova", "kova.exe", "kova-service.exe", "mihomo", "mihomo.exe", "xray", "xray.exe"];

/// Unique program names, sorted case-insensitively.
pub fn running() -> Vec<String> {
    let mut names: Vec<String> = imp::running().into_iter().filter(|n| !n.is_empty() && !OWN.contains(&n.to_ascii_lowercase().as_str())).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};

    /// The full path of process `pid`, if this user may look at it.
    fn path_of(pid: u32) -> Option<String> {
        // SAFETY: a handle opened here and closed below; the buffer outlives the call.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(h);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    pub fn running() -> Vec<String> {
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
        let mut out = Vec::new();
        // SAFETY: a snapshot handle walked with an entry of the size it expects, then closed.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut e) != 0;
            while more {
                // Processes of other accounts and the system can't be opened, and the
                // ones from the Windows folder are Windows' own.
                if let Some(path) = path_of(e.th32ProcessID).filter(|p| !p.to_lowercase().starts_with(&windows)) {
                    if let Some(name) = std::path::Path::new(&path).file_name() {
                        out.push(name.to_string_lossy().into_owned());
                    }
                }
                more = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
        out
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn running() -> Vec<String> {
        // This user's processes with their paths; macOS's own are left out.
        let out = std::process::Command::new("ps").args(["-x", "-o", "comm="]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        out.lines()
            .map(str::trim)
            .filter(|p| !p.starts_with("/System/") && !p.starts_with("/usr/") && !p.starts_with("/sbin/") && !p.starts_with("/bin/"))
            .filter_map(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    pub fn running() -> Vec<String> {
        // `/proc/<pid>/exe` can be read for this user's processes only.
        let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
        entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|e| std::fs::read_link(e.path().join("exe")).ok())
            .filter(|p| !p.starts_with("/usr/lib/") && !p.starts_with("/usr/libexec/") && !p.starts_with("/usr/sbin/"))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().trim_end_matches(" (deleted)").to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    /// This test's own program shows up, Kova's cores never do.
    #[test]
    fn lists_this_users_programs() {
        let names = super::running();
        let me = std::env::current_exe().unwrap();
        let me = me.file_name().unwrap().to_string_lossy();
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case(&me)), "{me} in {names:?}");
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("mihomo.exe") || n == "mihomo"));
    }
}
