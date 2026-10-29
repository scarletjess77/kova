use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use kova_core::CoreKind;
use kova_engine::process::CoreProcess;
use kova_engine::service::{PIPE_NAME, PROTOCOL_VERSION, Request, Response};
use kova_engine::{cores, geodata};

use crate::{EXIT_COPY, EXIT_NO_ADMIN, EXIT_REGISTER, EXIT_REMOVE, EXIT_START, Failure};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::Mutex;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

pub const SERVICE_NAME: &str = "KovaService";
const DISPLAY_NAME: &str = "Kova TUN service";
const MAX_REQUEST: u64 = 16 * 1024 * 1024;
const MAX_LOG_LINES: usize = 2000;

/// Where the service keeps its binary, its Mihomo and Mihomo's home. Under Program
/// Files, so ordinary users can read but not change anything there.
///
/// Taken from Windows' known-folder API, never from environment variables: those can
/// be set per user, and installing runs as administrator. A user-chosen folder here
/// would let any program running as the user replace the SYSTEM service's binary.
pub fn service_home() -> PathBuf {
    program_files().join("Kova Service")
}

fn program_files() -> PathBuf {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath};
    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    let ok = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, std::ptr::null_mut(), &mut raw) } == 0;
    let path = if ok && !raw.is_null() {
        let len = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
        let wide = unsafe { std::slice::from_raw_parts(raw, len) };
        Some(PathBuf::from(String::from_utf16_lossy(wide)))
    } else {
        None
    };
    unsafe { CoTaskMemFree(raw as _) };
    path.unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

// ------------------------------------------------------------------------------- server

/// A process handle that closes itself; used to notice the app exiting.
struct Owner(HANDLE);
unsafe impl Send for Owner {}
impl Owner {
    fn open(pid: u32) -> Option<Self> {
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        (!h.is_null()).then_some(Self(h))
    }
    fn exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Default)]
struct Logs {
    seq: u64,
    lines: VecDeque<(u64, String)>,
}

struct Service {
    home: PathBuf,
    /// Start requests are handled one at a time.
    starting: Mutex<()>,
    core: Mutex<Option<CoreProcess>>,
    owner: Mutex<Option<Owner>>,
    exit: StdMutex<Option<String>>,
    logs: Arc<StdMutex<Logs>>,
}

impl Service {
    fn cores_dir(&self) -> PathBuf {
        self.home.join("cores")
    }
    fn run_dir(&self) -> PathBuf {
        self.home.join("run")
    }

    fn log(&self, line: &str) {
        push_log(&self.logs, line);
    }

    async fn stop_core(&self, reason: Option<&str>) {
        *self.owner.lock().await = None;
        if let Some(c) = self.core.lock().await.take() {
            c.stop().await;
            if let Some(r) = reason {
                self.log(r);
                *self.exit.lock().unwrap() = Some(r.to_string());
            }
        }
    }

    async fn handle(&self, req: Request) -> Response {
        match req {
            Request::Hello => Response {
                version: Some(env!("CARGO_PKG_VERSION").into()),
                protocol: Some(PROTOCOL_VERSION),
                core_version: cores::installed_version(&self.cores_dir(), CoreKind::Mihomo),
                // Hashed on first use, off the async threads (it reads the whole exe).
                exe_sha256: tokio::task::spawn_blocking(kova_engine::service::own_exe_sha256).await.ok().flatten(),
                ..Response::ok()
            },
            Request::InstallCore => match cores::install_latest(&self.cores_dir(), CoreKind::Mihomo).await {
                Ok(tag) => Response { core_version: Some(tag), ..Response::ok() },
                Err(e) => Response::err(e),
            },
            Request::Start { config, geodata: source, owner_pid } => match self.start_one(&config, &source, owner_pid).await {
                Ok(()) => Response { running: true, ..Response::ok() },
                Err(e) => Response::err(e),
            },
            Request::Stop => {
                self.stop_core(None).await;
                Response::ok()
            }
            Request::Status => {
                let mut core = self.core.lock().await;
                let exited = core.as_mut().and_then(|c| c.exited());
                if let Some(msg) = exited {
                    core.take();
                    *self.exit.lock().unwrap() = Some(msg);
                }
                let log_seq = self.logs.lock().unwrap().seq;
                Response { running: core.is_some(), exit: self.exit.lock().unwrap().clone(), log_seq, ..Response::ok() }
            }
            Request::Logs { since } => {
                let logs = self.logs.lock().unwrap();
                Response { lines: logs.lines.iter().filter(|(n, _)| *n > since).take(500).cloned().collect(), log_seq: logs.seq, ..Response::ok() }
            }
        }
    }

    async fn start_one(&self, config: &str, source: &kova_core::options::GeoSource, owner_pid: u32) -> Result<(), String> {
        let _one_at_a_time = self.starting.lock().await;
        self.start(config, source, owner_pid).await
    }

    async fn start(&self, config: &str, source: &kova_core::options::GeoSource, owner_pid: u32) -> Result<(), String> {
        let config = kova_core::gen_mihomo::sanitize_for_service(config).map_err(|e| e.to_string())?;
        let owner = Owner::open(owner_pid).ok_or("the requesting app is not running")?;
        let cores_dir = self.cores_dir();
        let bin = cores::binary_path(&cores_dir, CoreKind::Mihomo);
        if !bin.exists() {
            self.log("Downloading Mihomo for the Kova service…");
            let tag = cores::install_latest(&cores_dir, CoreKind::Mihomo).await.map_err(|e| e.to_string())?;
            self.log(&format!("Mihomo {tag} installed"));
        }
        // Routing data: our own verified copy of the app's chosen source. Downloaded only
        // when missing or when the source changed, so connecting is never held up by a
        // refresh; Mihomo's own geo-auto-update keeps the copy current.
        match geodata::ensure(&cores_dir, source, None).await {
            Ok((_, true)) => self.log("Downloaded routing data for the Kova service"),
            Ok((_, false)) => {}
            // Not fatal: Mihomo can still fetch its own copy from geox-url.
            Err(e) => self.log(&format!("routing data download failed: {e}")),
        }

        self.stop_core(None).await;
        *self.exit.lock().unwrap() = None;
        let run = self.run_dir();
        std::fs::create_dir_all(&run).map_err(|e| e.to_string())?;
        geodata::seed_mihomo_home(&cores_dir, &run);
        let cfg = run.join("config.yaml");
        std::fs::write(&cfg, config).map_err(|e| e.to_string())?;
        let logs = self.logs.clone();
        let sink: kova_engine::process::LineSink = Arc::new(move |_core: &str, line: &str| push_log(&logs, line));
        let (h, c) = (run.to_string_lossy().to_string(), cfg.to_string_lossy().to_string());
        let mut proc = CoreProcess::spawn("mihomo", &bin, &["-d", &h, "-f", &c], &run, &[], sink).map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Some(msg) = proc.exited() {
            return Err(msg);
        }
        *self.core.lock().await = Some(proc);
        *self.owner.lock().await = Some(owner);
        Ok(())
    }
}

fn push_log(logs: &StdMutex<Logs>, line: &str) {
    let mut l = logs.lock().unwrap();
    l.seq += 1;
    let n = l.seq;
    l.lines.push_back((n, line.to_string()));
    if l.lines.len() > MAX_LOG_LINES {
        l.lines.pop_front();
    }
}

/// A pipe instance only SYSTEM, administrators and interactively logged-on users may open.
fn create_pipe(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)\0".encode_utf16().collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let pipe = unsafe {
        ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut _)
    };
    unsafe { LocalFree(sd as _) };
    pipe
}

/// The programs that may use the service: the Kova that installed it. Kept in the
/// service's folder, which only administrators can change.
const CLIENTS: &str = "clients.txt";

/// Whether the program at the other end of `pipe` may use the service. The pipe's
/// security lets in any signed-in user; this narrows it to Kova itself, so another
/// program can't have Mihomo run with system rights. Without the list (the service
/// run from a console, or installed by an older Kova until it is reinstalled) anyone
/// the pipe lets in may.
fn allowed_client(home: &Path, pipe: &NamedPipeServer) -> bool {
    let Ok(list) = std::fs::read_to_string(home.join(CLIENTS)) else { return true };
    let Some(image) = client_image(pipe) else { return false };
    list.lines().map(str::trim).any(|l| !l.is_empty() && same_path(l, &image))
}

fn same_path(a: &str, b: &str) -> bool {
    let plain = |s: &str| s.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase();
    plain(a) == plain(b)
}

/// The full path of the program at the other end of `pipe`.
fn client_image(pipe: &NamedPipeServer) -> Option<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    use windows_sys::Win32::System::Threading::{PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    let mut pid = 0u32;
    // SAFETY: a live pipe handle and a u32 to write to.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        return None;
    }
    // SAFETY: plain calls on a process handle closed below; the buffer outlives them.
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

async fn serve_connection(mut conn: NamedPipeServer, svc: Arc<Service>) {
    if !allowed_client(&svc.home, &conn) {
        let mut out = serde_json::to_string(&Response::err("Only Kova may use the Kova service")).expect("json");
        out.push('\n');
        let _ = conn.write_all(out.as_bytes()).await;
        svc.log("refused a program that is not Kova");
        return;
    }
    let (r, mut w) = tokio::io::split(&mut conn);
    let mut line = String::new();
    // A client that connects and never writes must not hold a task forever.
    let read = tokio::time::timeout(Duration::from_secs(10), BufReader::new(r.take(MAX_REQUEST)).read_line(&mut line)).await;
    let resp = match read {
        Err(_) | Ok(Ok(0)) | Ok(Err(_)) => return,
        Ok(Ok(_)) => match serde_json::from_str::<Request>(&line) {
            Ok(req) => svc.handle(req).await,
            Err(e) => Response::err(format!("bad request: {e}")),
        },
    };
    let mut out = serde_json::to_string(&resp).expect("json");
    out.push('\n');
    let _ = w.write_all(out.as_bytes()).await;
    let _ = w.flush().await;
}

/// Serve requests on `pipe` until `shutdown` resolves, then stop Mihomo.
pub async fn serve(pipe: String, home: PathBuf, shutdown: impl std::future::Future<Output = ()>) -> std::io::Result<()> {
    let pipe = pipe.as_str();
    std::fs::create_dir_all(&home)?;
    let svc = Arc::new(Service {
        home,
        starting: Mutex::new(()),
        core: Mutex::new(None),
        owner: Mutex::new(None),
        exit: StdMutex::new(None),
        logs: Default::default(),
    });

    // Stop Mihomo when the app that started it goes away.
    let watcher = {
        let svc = svc.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let gone = svc.owner.lock().await.as_ref().is_some_and(Owner::exited);
                if gone {
                    svc.stop_core(Some("Kova closed, so the service stopped Mihomo")).await;
                }
            }
        })
    };

    // Failing to create the very first instance means we can't serve at all. After that,
    // a single bad connection or a transient error must never stop the service.
    let mut server = create_pipe(pipe, true)?;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            r = server.connect() => {
                let next = next_instance(pipe).await;
                let conn = std::mem::replace(&mut server, next);
                match r {
                    Ok(()) => {
                        tokio::spawn(serve_connection(conn, svc.clone()));
                    }
                    Err(e) => svc.log(&format!("pipe connection failed: {e}")),
                }
            }
            _ = &mut shutdown => break,
        }
    }
    watcher.abort();
    svc.stop_core(None).await;
    Ok(())
}

/// A new pipe instance, retrying until Windows hands one out.
async fn next_instance(pipe: &str) -> NamedPipeServer {
    loop {
        match create_pipe(pipe, false) {
            Ok(p) => return p,
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

// ------------------------------------------------------------------------------ service

define_windows_service!(ffi_service_main, service_main);

pub fn run_as_service() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else { return };
    let set = |state: ServiceState, code: u32| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running { ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN } else { ServiceControlAccept::empty() },
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };
    set(ServiceState::Running, 0);
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut rx = stop_rx;
    let result = runtime.block_on(serve(PIPE_NAME.to_string(), service_home(), async move {
        let _ = rx.wait_for(|stop| *stop).await;
    }));
    set(ServiceState::Stopped, u32::from(result.is_err()));
}

/// Run the pipe server in the foreground (development and tests). Stops on Ctrl+C.
pub fn run_console() -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    // Only the console mode may use another folder: it isn't elevated and isn't the
    // installed service.
    let home = std::env::var_os("KOVA_SERVICE_HOME").map(PathBuf::from).unwrap_or_else(service_home);
    runtime
        .block_on(serve(kova_engine::service::pipe_name(), home, async {
            let _ = tokio::signal::ctrl_c().await;
        }))
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------------- installation

fn wait_stopped(svc: &windows_service::service::Service) {
    for _ in 0..50 {
        match svc.query_status() {
            Ok(s) if s.current_state == ServiceState::Stopped => return,
            Err(_) => return,
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Copy this executable into the protected service folder, register the service and
/// start it. Needs administrator rights; safe to run again to update.
pub fn install() -> Result<(), Failure> {
    let home = service_home();
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| Failure(EXIT_NO_ADMIN, format!("administrator rights are needed ({e})")))?;
    std::fs::create_dir_all(&home).map_err(|e| Failure(EXIT_COPY, format!("could not create {}: {e}", home.display())))?;
    let target = home.join("kova-service.exe");
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG;
    let existing = manager.open_service(SERVICE_NAME, access).ok();
    if let Some(svc) = &existing {
        let _ = svc.stop();
        wait_stopped(svc);
    }

    let me = std::env::current_exe().map_err(|e| Failure(EXIT_COPY, e.to_string()))?;
    if !same_file(&me, &target) {
        let mut copied = Err(String::new());
        for _ in 0..20 {
            copied = std::fs::copy(&me, &target).map(drop).map_err(|e| format!("could not copy the service: {e}"));
            if copied.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        copied.map_err(|e| Failure(EXIT_COPY, e))?;
    }
    // Only the Kova that installs the service may use it (see `allowed_client`).
    std::fs::write(home.join(CLIENTS), me.display().to_string()).map_err(|e| Failure(EXIT_COPY, format!("could not write {}: {e}", home.join(CLIENTS).display())))?;
    // Mihomo and the routing data that came with the installer, so TUN works without
    // GitHub, which is blocked in some countries. Only from Kova's own folder under
    // Program Files, which ordinary programs can't change.
    if let Some(bundled) = me.parent().map(|d| d.join("cores")).filter(|d| d.starts_with(program_files())) {
        let cores = home.join("cores");
        let wanted = |n: &str| n == cores::binary_name(CoreKind::Mihomo) || n.ends_with(".dat");
        if let Ok(entries) = std::fs::read_dir(&bundled) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if wanted(&name) && !cores.join(&name).exists() {
                    let _ = std::fs::create_dir_all(&cores);
                    let _ = std::fs::copy(e.path(), cores.join(&name));
                }
            }
        }
    }

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target,
        launch_arguments: vec![OsString::from("--service-run")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let svc = match existing {
        Some(svc) => {
            svc.change_config(&info).map_err(|e| Failure(EXIT_REGISTER, e.to_string()))?;
            svc
        }
        None => manager.create_service(&info, access).map_err(|e| Failure(EXIT_REGISTER, e.to_string()))?,
    };
    let _ = svc.set_description("Runs Mihomo with the rights Kova's TUN mode needs.");
    let _ = svc.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) }; 3]),
    });
    firewall(true);
    svc.start(&[] as &[&OsStr]).map_err(|e| Failure(EXIT_START, format!("the service was installed but did not start: {e}")))
}

/// The Windows Firewall rule for the service's Mihomo.
const FIREWALL_RULE: &str = "Kova TUN service";

/// Windows Firewall asks before a program may receive connections, but a service has no
/// window to ask in, so TUN's `system` and `mixed` stacks would never see the traffic
/// they capture. Allow the service's own Mihomo, which ordinary programs can't replace
/// (see [`service_home`]); the rule goes with the service. Not fatal: the `gvisor` stack
/// works without it.
fn firewall(allow: bool) {
    use std::os::windows::process::CommandExt;
    let netsh = |args: &[String]| {
        let mut c = std::process::Command::new("netsh");
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        for a in args {
            c.raw_arg(a);
        }
        c.output().map(|o| o.status.success()).unwrap_or(false)
    };
    let name = format!("name=\"{FIREWALL_RULE}\"");
    // Deleting first keeps one rule however often the service is reinstalled.
    netsh(&["advfirewall".into(), "firewall".into(), "delete".into(), "rule".into(), name.clone()]);
    if !allow {
        return;
    }
    let program = format!("program=\"{}\"", service_home().join("cores").join(cores::binary_name(CoreKind::Mihomo)).display());
    let added = netsh(&["advfirewall", "firewall", "add", "rule", &name, "dir=in", "action=allow", &program, "enable=yes", "profile=any"].map(String::from));
    if !added {
        eprintln!("could not add the Windows Firewall rule for TUN; the gvisor stack works without it");
    }
}

/// Stop and remove the service and its folder. Needs administrator rights.
pub fn uninstall() -> Result<(), Failure> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| Failure(EXIT_NO_ADMIN, e.to_string()))?;
    if let Ok(svc) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        let _ = svc.stop();
        wait_stopped(&svc);
        svc.delete().map_err(|e| Failure(EXIT_REMOVE, format!("could not remove the service: {e}")))?;
    }
    firewall(false);
    let home = service_home();
    for _ in 0..20 {
        if std::fs::remove_dir_all(&home).is_ok() || !home.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// `Some(running)` if the service is installed, `None` if not. Works without admin rights.
pub fn query() -> Option<bool> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let svc = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).ok()?;
    Some(svc.query_status().ok()?.current_state == ServiceState::Running)
}

/// Run this executable with `arg` as administrator (shows the UAC prompt) and wait for
/// it. `Ok(exit code)`, or an error if the user declined.
pub fn run_elevated(arg: &str) -> Result<i32, String> {
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let wide = |s: &OsStr| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(std::iter::once(0)).collect()
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (verb, file, params) = (wide(OsStr::new("runas")), wide(exe.as_os_str()), wide(OsStr::new(arg)));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = std::io::Error::last_os_error();
        return Err(if e.raw_os_error() == Some(1223) { "The administrator prompt was declined".into() } else { e.to_string() });
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    Ok(code as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::windows::named_pipe::ClientOptions;

    /// A pipe with this test as the client, and the service's view of it.
    async fn connected(name: &str) -> (NamedPipeServer, tokio::net::windows::named_pipe::NamedPipeClient) {
        let server = create_pipe(name, true).unwrap();
        let client = ClientOptions::new().open(name).unwrap();
        server.connect().await.unwrap();
        (server, client)
    }

    #[tokio::test]
    async fn only_the_kova_that_installed_the_service_may_use_it() {
        let home = tempfile::tempdir().unwrap();
        let name = format!(r"\\.\pipe\kova-client-test-{}", std::process::id());
        let (server, _client) = connected(&name).await;
        let me = std::env::current_exe().unwrap();
        assert!(allowed_client(home.path(), &server), "no list yet: as before");

        std::fs::write(home.path().join(CLIENTS), format!("{}\n", me.display().to_string().to_uppercase())).unwrap();
        assert!(allowed_client(home.path(), &server), "the listed program, whatever the case of its path");

        std::fs::write(home.path().join(CLIENTS), r"C:\Program Files\Kova\kova.exe").unwrap();
        assert!(!allowed_client(home.path(), &server), "any other program is refused");
    }
}
