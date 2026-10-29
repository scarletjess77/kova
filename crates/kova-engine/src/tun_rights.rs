//! The parts of granting Linux TUN rights (see `tun_linux`) that don't touch the system,
//! kept apart so they are checked on every platform's test run.

/// The root script that installs the copy of Mihomo with network rights. `$1` is the
/// Mihomo to copy, `$2` the user allowed to run the copy: the one who said yes.
///
/// File capabilities apply to whoever runs a file, so a copy anyone may run would hand
/// network-admin rights to every account on the machine (with a config of its own:
/// routes, firewall, capturing traffic). Not the user's group either: on some systems
/// every account shares one. Owned by the user, group root, run by the owner only; the
/// owner can't change its content without the capability going (the kernel drops it on
/// a write), and the folder is root's, so the file can't be swapped.
pub fn grant_script(target: &str, caps: &str) -> String {
    format!(
        "set -e; case \"$2\" in ''|*[!0-9]*) exit 2;; esac; \
         install -D -m 0500 -o \"$2\" -g 0 \"$1\" {target}; \
         setcap {caps} {target} || {{ rm -f {target}; exit 1; }}"
    )
}

/// Whether a file with this mode can be run by accounts other than its owner.
pub fn open_to_others(mode: u32) -> bool {
    mode & 0o077 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_for_the_user_only() {
        let s = grant_script("/usr/local/lib/kova/mihomo", "cap_net_admin+ep");
        assert!(s.contains("install -D -m 0500 -o \"$2\" -g 0 \"$1\" /usr/local/lib/kova/mihomo"), "{s}");
        assert!(s.contains("setcap cap_net_admin+ep /usr/local/lib/kova/mihomo || { rm -f /usr/local/lib/kova/mihomo; exit 1; }"), "{s}");
        assert!(s.starts_with("set -e; case \"$2\" in ''|*[!0-9]*) exit 2;; esac;"), "only a numeric user is accepted: {s}");

        assert!(open_to_others(0o755), "the first copies were runnable by anyone");
        assert!(open_to_others(0o750), "the next ones by the user's group");
        assert!(!open_to_others(0o500));
        assert!(!open_to_others(0o100500), "with the file-type bits too");
    }
}
