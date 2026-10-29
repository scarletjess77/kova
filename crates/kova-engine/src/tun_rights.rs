//! The parts of granting Linux TUN rights (see `tun_linux`) that don't touch the system,
//! kept apart so they are checked on every platform's test run.

/// The root script that installs the copy of Mihomo with network rights. `$1` is the
/// Mihomo to copy, `$2` the group allowed to run the copy: the user's who said yes.
///
/// File capabilities apply to whoever runs a file, so a copy anyone may run would hand
/// network-admin rights to every account on the machine (with a config of its own:
/// routes, firewall, capturing traffic). Owner root, group the user's, nobody else.
pub fn grant_script(target: &str, caps: &str) -> String {
    format!(
        "set -e; case \"$2\" in ''|*[!0-9]*) exit 2;; esac; \
         install -D -m 0750 -o root -g \"$2\" \"$1\" {target}; \
         setcap {caps} {target} || {{ rm -f {target}; exit 1; }}"
    )
}

/// Whether a file with this mode can be run by accounts outside its owner and group.
pub fn open_to_others(mode: u32) -> bool {
    mode & 0o007 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_for_root_and_the_users_group_only() {
        let s = grant_script("/usr/local/lib/kova/mihomo", "cap_net_admin+ep");
        assert!(s.contains("install -D -m 0750 -o root -g \"$2\" \"$1\" /usr/local/lib/kova/mihomo"), "{s}");
        assert!(s.contains("setcap cap_net_admin+ep /usr/local/lib/kova/mihomo || { rm -f /usr/local/lib/kova/mihomo; exit 1; }"), "{s}");
        assert!(s.starts_with("set -e; case \"$2\" in ''|*[!0-9]*) exit 2;; esac;"), "only a numeric group is accepted: {s}");

        assert!(open_to_others(0o755), "the old copies were runnable by anyone");
        assert!(open_to_others(0o751));
        assert!(!open_to_others(0o750));
        assert!(!open_to_others(0o100750), "with the file-type bits too");
    }
}
