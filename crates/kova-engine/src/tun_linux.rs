//! Linux: TUN without running Kova as root. Creating the virtual adapter and its routes
//! needs the network-admin capability. Kova asks once (polkit's password prompt) to put a
//! copy of its Mihomo in a root-owned folder with only that capability, and runs TUN with
//! the copy. Nothing else gets elevated, and the copy can't be swapped by the user: the
//! folder belongs to root, and a changed file loses its capability.

use std::path::{Path, PathBuf};

use crate::Error;

const DIR: &str = "/usr/local/lib/kova";
const CAPS: &str = "cap_net_admin,cap_net_bind_service,cap_net_raw+ep";

/// Where the copy with network rights lives.
pub fn mihomo_path() -> PathBuf {
    Path::new(DIR).join("mihomo")
}

/// The copy, if it is there, owned by root, still carries its capabilities, and only this
/// user's group may run it. A copy anyone may run (as earlier versions granted) doesn't
/// count: TUN asks for the rights again, which replaces it.
pub fn granted() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let p = mihomo_path();
    // SAFETY: getgid has no preconditions.
    let gid = unsafe { libc::getgid() };
    std::fs::metadata(&p)
        .ok()
        .filter(|m| m.is_file() && m.uid() == 0 && m.gid() == gid && !crate::tun_rights::open_to_others(m.mode()) && has_caps(&p))
        .map(|_| p)
}

/// Whether `path` has file capabilities (the `security.capability` attribute setcap
/// writes; a changed file loses it).
fn has_caps(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(file) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: both strings are NUL-terminated; a null buffer of size 0 only asks for the
    // attribute's length.
    unsafe { libc::getxattr(file.as_ptr(), c"security.capability".as_ptr(), std::ptr::null_mut(), 0) > 0 }
}

pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// The copy differs from the Mihomo Kova keeps up to date, so it needs granting again.
pub fn outdated(current: &Path) -> bool {
    let hash = |p: &Path| std::fs::read(p).ok().map(|b| crate::fetch::sha256_hex(&b));
    match (granted(), hash(current)) {
        (Some(copy), Some(ours)) => hash(&copy).is_some_and(|theirs| theirs != ours),
        _ => false,
    }
}

/// Run `script` as root through polkit, with `args` as `$1`, `$2`…
fn as_root(script: &str, args: &[&str]) -> Result<(), Error> {
    let status = std::process::Command::new("pkexec")
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .args(args)
        .status()
        .map_err(|e| Error::Other(format!("could not ask for administrator rights (pkexec: {e}). Install polkit, or run: sudo setcap {CAPS} <Mihomo>")))?;
    match status.code() {
        Some(0) => Ok(()),
        // pkexec: the dialog was dismissed, or the user isn't allowed.
        Some(126) | Some(127) => Err(Error::Other("Administrator rights were not given".into())),
        _ => Err(Error::Other(format!("granting TUN rights failed ({status}); is setcap (libcap) installed?"))),
    }
}

/// Copy `mihomo` to the root-owned folder and give the copy network rights.
pub fn grant(mihomo: &Path) -> Result<(), Error> {
    if !mihomo.exists() {
        return Err(Error::Other("mihomo is not installed".into()));
    }
    // A copy without its capability would only look granted: the script takes it away on
    // failure. Only this user's group may run the copy (see `tun_rights::grant_script`).
    let script = crate::tun_rights::grant_script(&mihomo_path().to_string_lossy(), CAPS);
    // SAFETY: getgid has no preconditions.
    let gid = unsafe { libc::getgid() }.to_string();
    as_root(&script, &[&mihomo.to_string_lossy(), &gid])?;
    if granted().is_none() {
        return Err(Error::Other("the network rights didn't stick (setcap); is the folder on a file system without capabilities?".into()));
    }
    Ok(())
}

/// Remove the copy.
pub fn revoke() -> Result<(), Error> {
    as_root(&format!("rm -f {}", mihomo_path().display()), &[])
}
