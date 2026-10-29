//! Per-workspace app runtime: child processes and static loopback servers.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::net::{TcpListener as StdTcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tower_http::services::{ServeDir, ServeFile};

use super::exec;
use super::registry::{AppRegistry, RegisteredApp};
use super::schema::{AppStatus, LaunchSpec, PortSpec};
use crate::config::path::strip_verbatim;
use crate::serve::shutdown::is_process_alive;

/// Everything a launch may substitute. Bundled apps also have an install
/// directory and a data directory; a registered app only has the workspace.
pub struct LaunchContext {
    pub workspace: PathBuf,
    pub app_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl LaunchContext {
    pub fn registered(workspace: PathBuf) -> Self {
        Self {
            workspace,
            app_dir: None,
            data_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeView {
    pub status: AppStatus,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
}

impl RuntimeView {
    pub fn idle() -> Self {
        Self {
            status: AppStatus::Stopped,
            port: None,
            url: None,
            error: None,
            started_at: None,
        }
    }
}

/// Last output lines of an app process, kept in memory. LiteCode does not own
/// the app's log files: the app writes wherever it wants, and this tail only
/// exists to explain a crash in the panel.
#[derive(Clone, Default)]
pub struct OutputTail(Arc<Mutex<VecDeque<String>>>);

const OUTPUT_TAIL_LINES: usize = 120;

impl OutputTail {
    fn push_line(&self, line: String) {
        let Ok(mut lines) = self.0.lock() else { return };
        if lines.len() == OUTPUT_TAIL_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    pub fn text(&self) -> String {
        self.0
            .lock()
            .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

enum RuntimeKind {
    Process {
        child: Child,
        pid: u32,
        output: OutputTail,
    },
    Static {
        shutdown: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    },
}

struct RuntimeState {
    status: AppStatus,
    port: Option<u16>,
    url: Option<String>,
    error: Option<String>,
    started_at: Option<i64>,
    kind: Option<RuntimeKind>,
    run_marker: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunMarker {
    workspace: String,
    pid: u32,
}

impl RuntimeState {
    fn idle(status: AppStatus) -> Self {
        Self {
            status,
            port: None,
            url: None,
            error: None,
            started_at: None,
            kind: None,
            run_marker: None,
        }
    }
}

pub struct AppRuntime {
    registry: Arc<AppRegistry>,
    states: Mutex<HashMap<String, RuntimeState>>,
}

impl AppRuntime {
    pub fn new(registry: Arc<AppRegistry>) -> Self {
        Self {
            registry,
            states: Mutex::new(HashMap::new()),
        }
    }

    pub fn view(&self, app_id: &str) -> RuntimeView {
        let mut states = self.states.lock().expect("app runtime lock");
        let state = states
            .entry(app_id.to_string())
            .or_insert_with(|| RuntimeState::idle(AppStatus::Stopped));
        if let Some(RuntimeKind::Process { child, output, .. }) = state.kind.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.status = AppStatus::Crashed;
                    state.error = Some(crash_message(status.code(), output));
                    state.kind = None;
                    state.url = None;
                    if let Some(path) = state.run_marker.take() {
                        let _ = std::fs::remove_file(path);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    state.status = AppStatus::Crashed;
                    state.error = Some(format!("cannot inspect app process: {err}"));
                    state.kind = None;
                    state.url = None;
                    if let Some(path) = state.run_marker.take() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
        RuntimeView {
            status: state.status,
            port: state.port,
            url: state.url.clone(),
            error: state.error.clone(),
            started_at: state.started_at,
        }
    }

    pub fn status(&self, app_id: &str) -> AppStatus {
        self.view(app_id).status
    }

    pub fn stop(&self, app_id: &str) {
        let mut states = self.states.lock().expect("app runtime lock");
        let Some(state) = states.get_mut(app_id) else {
            return;
        };
        state.status = AppStatus::Stopping;
        let run_marker = state.run_marker.take();
        match state.kind.take() {
            Some(RuntimeKind::Process { mut child, pid, .. }) => {
                crate::terminal::kill_process_tree(pid);
                let _ = child.wait();
            }
            Some(RuntimeKind::Static { shutdown, task }) => {
                if let Some(tx) = shutdown {
                    let _ = tx.send(());
                }
                task.abort();
            }
            None => {}
        }
        state.status = AppStatus::Stopped;
        state.port = None;
        state.url = None;
        state.error = None;
        state.started_at = None;
        if let Some(path) = run_marker {
            let _ = std::fs::remove_file(path);
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<String> = self
            .states
            .lock()
            .map(|states| states.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            self.stop(&id);
        }
    }

    pub fn start_process(
        &self,
        app: &RegisteredApp,
        ctx: &LaunchContext,
    ) -> Result<RuntimeView, String> {
        let LaunchSpec::Process {
            command,
            args,
            cwd,
            env,
            port,
            url,
            ready_path,
            timeout_ms,
        } = &app.definition.launch
        else {
            return Err("app does not use a process launch".into());
        };
        {
            let states = self.states.lock().expect("app runtime lock");
            if states
                .get(&app.id)
                .map(|s| {
                    matches!(
                        s.status,
                        AppStatus::Running | AppStatus::Starting | AppStatus::Stopping
                    )
                })
                .unwrap_or(false)
            {
                return Err("app is already running".into());
            }
        }
        let port = allocate_port(port)?;
        let workdir = resolve_workdir(&expand(cwd, ctx, port)?, ctx)?;
        let expanded_url = expand(url, ctx, port)?;
        let program = exec::resolve_executable(&expand(command, ctx, port)?, &workdir)
            .map_err(|err| format!("{err} (cwd: {})", workdir.display()))?;
        let mut all_args = Vec::with_capacity(args.len());
        for arg in args {
            all_args.push(expand(arg, ctx, port)?);
        }
        let mut resolved_env = HashMap::new();
        for (key, value) in env {
            resolved_env.insert(key.clone(), expand(value, ctx, port)?);
        }

        let output = OutputTail::default();
        let mut cmd = Command::new(&program);
        cmd.args(&all_args)
            .current_dir(&workdir)
            .envs(&resolved_env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        // App processes are long-lived. On Linux PR_SET_PDEATHSIG is tied to
        // the spawning thread, and this call runs on a retirable blocking
        // worker, so binding it here would kill a healthy app. Graceful
        // shutdown and the Windows Job Object cover the supported paths.
        let marker = claim_run_marker(app, &self.registry, std::process::id())?;
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                remove_run_marker(marker.as_deref());
                return Err(format!("start {}: {err}", program.display()));
            }
        };
        let pid = child.id();
        if let Some(stdout) = child.stdout.take() {
            pump(stdout, output.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, output.clone());
        }
        crate::proc_lifetime::bind_child_to_parent(pid);
        if let Some(path) = marker.as_deref() {
            if let Err(err) = write_run_marker(path, &self.registry, pid) {
                crate::terminal::kill_process_tree(pid);
                let _ = child.wait();
                remove_run_marker(marker.as_deref());
                return Err(err);
            }
        }

        let ready_client = ready_path.as_ref().map(|_| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1000))
                .build()
        });
        let ready_client = match ready_client.transpose() {
            Ok(client) => client,
            Err(err) => {
                crate::terminal::kill_process_tree(pid);
                let _ = child.wait();
                remove_run_marker(marker.as_deref());
                return Err(format!("ready client: {err}"));
            }
        };
        let started = Instant::now();
        let ready = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break Err(crash_message(status.code(), &output));
            }
            let tcp_ready = TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}")
                    .parse()
                    .map_err(|_| "invalid app port".to_string())?,
                Duration::from_millis(300),
            )
            .is_ok();
            if tcp_ready {
                let http_ready = match (ready_path.as_deref(), ready_client.as_ref()) {
                    (Some(path), Some(client)) => client
                        .get(format!("http://127.0.0.1:{port}{path}"))
                        .send()
                        .map(|response| {
                            response.status().is_success() || response.status().is_redirection()
                        })
                        .unwrap_or(false),
                    _ => true,
                };
                if http_ready {
                    break Ok(());
                }
            }
            if started.elapsed() > Duration::from_millis(*timeout_ms) {
                break Err(match ready_path {
                    Some(path) => format!("app did not respond on 127.0.0.1:{port}{path} in time"),
                    None => format!("app did not listen on 127.0.0.1:{port} in time"),
                });
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let ready = ready.and_then(|()| {
            if listens_on_lan(port) {
                Err("app is listening on a non-loopback interface".to_string())
            } else {
                Ok(())
            }
        });
        if let Err(err) = ready {
            crate::terminal::kill_process_tree(pid);
            let _ = child.wait();
            remove_run_marker(marker.as_deref());
            let message = with_tail(err, &output);
            let mut states = self.states.lock().expect("app runtime lock");
            states.insert(
                app.id.clone(),
                RuntimeState {
                    status: AppStatus::Error,
                    port: None,
                    url: None,
                    error: Some(message.clone()),
                    started_at: None,
                    kind: None,
                    run_marker: None,
                },
            );
            return Err(message);
        }

        {
            let mut states = self.states.lock().expect("app runtime lock");
            states.insert(
                app.id.clone(),
                RuntimeState {
                    status: AppStatus::Running,
                    port: Some(port),
                    url: Some(expanded_url.clone()),
                    error: None,
                    started_at: Some(now_ms()),
                    kind: Some(RuntimeKind::Process { child, pid, output }),
                    run_marker: marker,
                },
            );
        }
        Ok(self.view(&app.id))
    }

    pub async fn start_static(
        &self,
        app: &RegisteredApp,
        ctx: &LaunchContext,
    ) -> Result<RuntimeView, String> {
        let LaunchSpec::Static { dir, spa } = &app.definition.launch else {
            return Err("app does not use a static launch".into());
        };
        {
            let states = self.states.lock().expect("app runtime lock");
            if states
                .get(&app.id)
                .map(|s| {
                    matches!(
                        s.status,
                        AppStatus::Running | AppStatus::Starting | AppStatus::Stopping
                    )
                })
                .unwrap_or(false)
            {
                return Err("app is already running".into());
            }
        }
        let app_dir = resolve_workdir(&expand(dir, ctx, 0)?, ctx)?;
        if !app_dir.is_dir() {
            return Err(format!(
                "static directory is missing: {}",
                app_dir.display()
            ));
        }
        if !spa && !app_dir.join("index.html").is_file() {
            return Err(format!(
                "static directory has no index.html: {}",
                app_dir.display()
            ));
        }
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("bind static app: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("static app addr: {e}"))?;
        let port = addr.port();
        let token = uuid::Uuid::new_v4().to_string();
        let cookie_name = format!("litecode_app_{port}");
        let spa = *spa;
        let auth = StaticAuth {
            cookie_name: cookie_name.clone(),
            token: token.clone(),
            used: Arc::new(AtomicBool::new(false)),
        };
        let app_router = if spa {
            Router::new()
                .fallback_service(
                    ServeDir::new(&app_dir).fallback(ServeFile::new(app_dir.join("index.html"))),
                )
                .layer(middleware::from_fn_with_state(auth, static_auth))
        } else {
            Router::new()
                .fallback_service(ServeDir::new(&app_dir))
                .layer(middleware::from_fn_with_state(auth, static_auth))
        };
        let marker = claim_run_marker(app, &self.registry, std::process::id())?;
        let (tx, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app_router).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(err) = server.await {
                tracing::warn!(error = %err, "static app server stopped");
            }
        });
        let url = format!("http://{addr}/?token={token}");
        let mut states = self.states.lock().expect("app runtime lock");
        states.insert(
            app.id.clone(),
            RuntimeState {
                status: AppStatus::Running,
                port: Some(port),
                url: Some(url.clone()),
                error: None,
                started_at: Some(now_ms()),
                kind: Some(RuntimeKind::Static {
                    shutdown: Some(tx),
                    task,
                }),
                run_marker: marker,
            },
        );
        Ok(RuntimeView {
            status: AppStatus::Running,
            port: Some(port),
            url: Some(url),
            error: None,
            started_at: Some(now_ms()),
        })
    }
}

/// Run marker directory, keyed by app and workspace. LiteCode owns this, not
/// the app: it lives under the global data root, never in an install dir.
fn run_marker_dir(registry: &AppRegistry, app_id: &str) -> PathBuf {
    registry.global_data.join("app-runtime").join(app_id)
}

fn claim_run_marker(
    app: &RegisteredApp,
    registry: &AppRegistry,
    pid: u32,
) -> Result<Option<PathBuf>, String> {
    let dir = run_marker_dir(registry, &app.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app run dir: {e}"))?;
    let own = dir.join(format!("{}.json", registry.workspace_hash()));
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path == own || path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let Ok(marker) = serde_json::from_str::<RunMarker>(&text) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            if marker.workspace == registry.workspace_hash() {
                continue;
            }
            if is_process_alive(marker.pid) {
                if app.definition.single_instance {
                    return Err("this app is already running in another workspace".into());
                }
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // Every running app leaves a marker, not only single-instance ones: it is
    // how another workspace knows a shared runtime is still serving pages.
    write_run_marker(&own, registry, pid)?;
    Ok(Some(own))
}

/// True while any workspace still has a live app process for this app.
/// Dead markers are cleaned here, so a later start can proceed.
pub fn has_live_run_marker(registry: &AppRegistry, app_id: &str) -> bool {
    let dir = run_marker_dir(registry, app_id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    let mut live = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let Ok(marker) = serde_json::from_str::<RunMarker>(&text) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if is_process_alive(marker.pid) {
            live = true;
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    live
}

fn write_run_marker(path: &Path, registry: &AppRegistry, pid: u32) -> Result<(), String> {
    let marker = RunMarker {
        workspace: registry.workspace_hash().to_string(),
        pid,
    };
    let text = serde_json::to_vec(&marker).map_err(|e| format!("serialize run marker: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write run marker: {e}"))
}

fn remove_run_marker(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Clone)]
struct StaticAuth {
    cookie_name: String,
    token: String,
    used: Arc<AtomicBool>,
}

async fn static_auth(
    State(auth): State<StaticAuth>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let cookie = format!("{}={}", auth.cookie_name, auth.token);
    let authorized = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| cookies.split(';').any(|part| part.trim() == cookie))
        .unwrap_or(false);
    if authorized {
        return next.run(request).await;
    }
    let query = request.uri().query().unwrap_or_default();
    let mut token_ok = false;
    let mut rest: Vec<String> = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "token" && value == auth.token {
            token_ok = true;
        } else {
            rest.push(pair.to_string());
        }
    }
    if !token_ok || auth.used.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::UNAUTHORIZED,
            "app token is missing or already used",
        )
            .into_response();
    }
    let mut location = request.uri().path().to_string();
    if !rest.is_empty() {
        location.push('?');
        location.push_str(&rest.join("&"));
    }
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/",
        auth.cookie_name, auth.token
    );
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::SEE_OTHER;
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn listens_on_lan(port: u16) -> bool {
    let Ok(socket) = std::net::UdpSocket::bind(("0.0.0.0", 0)) else {
        return false;
    };
    if socket.connect(("8.8.8.8", 80)).is_err() {
        return false;
    }
    let Ok(local) = socket.local_addr() else {
        return false;
    };
    let std::net::IpAddr::V4(ip) = local.ip() else {
        return false;
    };
    if ip.is_loopback() {
        return false;
    }
    TcpStream::connect_timeout(&(ip, port).into(), Duration::from_millis(350)).is_ok()
}

fn allocate_port(spec: &PortSpec) -> Result<u16, String> {
    if let Some(port) = spec.fixed() {
        let listener = StdTcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("port {port} is not available: {e}"))?;
        drop(listener);
        return Ok(port);
    }
    let listener =
        StdTcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("allocate app port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("app port: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// Substitute the tokens LiteCode owns. A bundled app also gets its install
/// directory and data directory; a registered app gets neither.
fn expand(value: &str, ctx: &LaunchContext, port: u16) -> Result<String, String> {
    let mut out = value.replace(
        "{workspace}",
        &strip_verbatim(ctx.workspace.clone()).to_string_lossy(),
    );
    out = out.replace("{port}", &port.to_string());
    for (token, dir) in [("{appdir}", &ctx.app_dir), ("{data}", &ctx.data_dir)] {
        if !out.contains(token) {
            continue;
        }
        match dir {
            Some(dir) => out = out.replace(token, &dir.to_string_lossy()),
            None => {
                return Err(format!(
                    "{token} is only available to bundled apps; use {{workspace}} instead"
                ));
            }
        }
    }
    Ok(out)
}

/// Relative launch paths are relative to the workspace; absolute paths are
/// used as written. The directory must already exist: LiteCode does not
/// create the places where an app runs.
fn resolve_workdir(value: &str, ctx: &LaunchContext) -> Result<PathBuf, String> {
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.workspace.join(path)
    };
    if resolved.is_dir() {
        Ok(resolved)
    } else {
        Err(format!(
            "working directory does not exist: {}",
            resolved.display()
        ))
    }
}

fn pump<R: Read + Send + 'static>(mut reader: R, tail: OutputTail) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                        let line: Vec<u8> = pending.drain(..=index).collect();
                        tail.push_line(String::from_utf8_lossy(&line).trim_end().to_string());
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if !pending.is_empty() {
            tail.push_line(String::from_utf8_lossy(&pending).trim_end().to_string());
        }
    });
}

fn crash_message(code: Option<i32>, output: &OutputTail) -> String {
    let exit = match code {
        Some(code) => format!("app exited with code {code}"),
        None => "app exited".to_string(),
    };
    with_tail(exit, output)
}

fn with_tail(message: String, output: &OutputTail) -> String {
    let tail = output.text();
    if tail.is_empty() {
        message
    } else {
        format!("{message}\n{tail}")
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_apps_get_only_workspace_and_port() {
        let ctx = LaunchContext::registered(PathBuf::from("/tmp/work"));
        let out = expand("{workspace}/site:{port}", &ctx, 8080).expect("expand");
        assert!(out.ends_with("/site:8080"), "{out}");
        let err = expand("{appdir}", &ctx, 1).expect_err("appdir token");
        assert!(err.contains("{appdir}"), "{err}");
    }

    #[test]
    fn bundled_apps_get_their_own_directories() {
        let ctx = LaunchContext {
            workspace: PathBuf::from("/tmp/work"),
            app_dir: Some(PathBuf::from("/data/apps/demo/app")),
            data_dir: Some(PathBuf::from("/tmp/work/.litecode/apps/demo")),
        };
        assert_eq!(
            expand("{appdir}/run.{data}", &ctx, 1).expect("expand"),
            "/data/apps/demo/app/run./tmp/work/.litecode/apps/demo"
        );
    }

    #[test]
    fn relative_working_directories_resolve_under_the_workspace() {
        let dir = std::env::temp_dir().join(format!("litecode-runtime-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("site")).expect("temp dir");
        let ctx = LaunchContext::registered(dir.clone());
        let resolved = resolve_workdir("site", &ctx).expect("resolve");
        assert_eq!(resolved, dir.join("site"));
        let missing = resolve_workdir("nope", &ctx).expect_err("missing dir");
        assert!(missing.contains("does not exist"), "{missing}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
