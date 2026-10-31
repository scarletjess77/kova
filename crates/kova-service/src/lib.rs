//! The Kova service: runs Mihomo with the rights TUN mode needs, so the app itself can
//! stay an ordinary user process.
//!
//! The same `kova.exe` is the service: `--service-install` (elevated, once) copies the
//! executable to `%ProgramFiles%\Kova Service` and registers it; Windows then starts it
//! with `--service-run`. The app talks to it over the named pipe described in
//! `kova_engine::service`.
//!
//! What keeps this from being a privilege-escalation hole:
//! - it only runs its own Mihomo, downloaded and checksum-verified by the service into a
//!   folder ordinary users can't write;
//! - configs are sanitised (no web UI, no extra controller channels, controller on
//!   loopback with a secret), and Mihomo confines provider paths to its home folder;
//! - the pipe refuses remote clients and is created as the first instance; while the
//!   service is stopped another program could take the name, so the app checks the
//!   pipe belongs to SYSTEM or an administrator before sending anything;
//! - Mihomo is stopped when the app process that started it exits.

#[cfg(windows)]
mod imp;

#[cfg(windows)]
pub use imp::*;

/// Handle `--service-*` arguments. Returns the process exit code if one was given,
/// `None` to continue starting the app normally.
pub fn dispatch_cli() -> Option<i32> {
    let arg = std::env::args().nth(1)?;
    #[cfg(windows)]
    {
        let code = match arg.as_str() {
            "--service-run" => i32::from(imp::run_as_service().is_err()),
            "--service-install" => report(imp::install()),
            "--service-uninstall" => report(imp::uninstall()),
            "--service-console" => report(imp::run_console().map_err(|e| Failure(1, e))),
            _ => return None,
        };
        Some(code)
    }
    #[cfg(not(windows))]
    {
        let _ = arg;
        None
    }
}

/// A failed `--service-*` command: an exit code the app can explain, plus detail for a
/// console. The elevated process reports only through its exit code; writing a file for
/// the app would mean writing as administrator into a folder the user controls.
#[derive(Debug)]
pub struct Failure(pub i32, pub String);

pub const EXIT_NO_ADMIN: i32 = 10;
pub const EXIT_COPY: i32 = 11;
pub const EXIT_REGISTER: i32 = 12;
pub const EXIT_START: i32 = 13;
pub const EXIT_REMOVE: i32 = 20;

/// What a `--service-install` / `--service-uninstall` exit code means, for the app.
pub fn describe_exit(code: i32) -> String {
    match code {
        EXIT_NO_ADMIN => "Windows did not grant administrator rights".into(),
        EXIT_COPY => "Could not copy the service into Program Files".into(),
        EXIT_REGISTER => "Could not register the Windows service".into(),
        EXIT_START => "The service was installed but did not start".into(),
        EXIT_REMOVE => "Could not remove the Windows service".into(),
        other => format!("The service setup failed (code {other})"),
    }
}

#[cfg(windows)]
fn report(r: Result<(), Failure>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(Failure(code, detail)) => {
            eprintln!("{detail}");
            code
        }
    }
}
