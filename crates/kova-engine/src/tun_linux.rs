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

/// The copy, if it is there and owned by root (so it still has its capability).
pub fn granted() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let p = mihomo_path();
    std::fs::metadata(&p).ok().filter(|m| m.is_file() && m.uid() == 0).map(|_| p)
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
    let script = format!("set -e; install -D -m 0755 -o root -g root \"$1\" {target}; setcap {CAPS} {target}", target = mihomo_path().display());
    as_root(&script, &[&mihomo.to_string_lossy()])
}

/// Remove the copy.
pub fn revoke() -> Result<(), Error> {
    as_root(&format!("rm -f {}", mihomo_path().display()), &[])
}
