//! Per-workspace app runtime: child processes and static loopback servers.

use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpListener as StdTcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tower_http::services::{ServeDir, ServeFile};

use super::install::{self, now_ms};
use super::registry::{AppRegistry, RegisteredApp};
use super::schema::{AppStatus, LaunchSpec, PortSpec};
use crate::config::path::strip_verbatim;
use crate::serve::shutdown::is_process_alive;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeView {
    pub status: AppStatus,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
}

impl RuntimeView {
    pub fn installed() -> Self {
        Self {
            status: AppStatus::Installed,
            port: None,
            url: None,
            error: None,
            started_at: None,
        }
    }
}

enum RuntimeKind {
    Process {
        child: Child,
        pid: u32,
        log: PathBuf,
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
            .or_insert_with(|| RuntimeState::idle(AppStatus::Installed));
        if let Some(RuntimeKind::Process { child, log, .. }) = state.kind.as_mut() {
            let log_path = log.display().to_string();
            match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code();
                    state.status = AppStatus::Crashed;
                    state.error = Some(match code {
                        Some(code) => format!("app exited with code {code}; log: {log_path}"),
                        None => format!("app exited; log: {log_path}"),
                    });
                    state.kind = None;
                    state.url = None;
                    if let Some(path) = state.run_marker.take() {
                        let _ = std::fs::remove_file(path);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    state.status = AppStatus::Crashed;
                    state.error = Some(format!(
                        "cannot inspect app process: {err}; log: {log_path}"
                    ));
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
        state.status = AppStatus::Installed;
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
        install_dir: &Path,
        data_dir: &Path,
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
        let port = allocate_port(port.clone())?;
        let app_dir = install::app_dir(install_dir);
        let data = data_dir.to_path_buf();
        // External apps can legitimately have no files yet (the command lives
        // on PATH); the workdir still has to exist before spawn.
        std::fs::create_dir_all(&app_dir).map_err(|e| format!("create app dir: {e}"))?;
        std::fs::create_dir_all(&data).map_err(|e| format!("create app data dir: {e}"))?;
        let expanded_url = expand(url, &self.registry, &app_dir, &data, port);
        let workdir = match cwd.as_deref().unwrap_or("app") {
            "workspace" => self.registry.workspace_root.clone(),
            _ => app_dir.clone(),
        };
        let expanded_command = expand(command, &self.registry, &app_dir, &data, port);
        let (program, mut all_args) = resolve_program(&expanded_command, &app_dir)?;
        for arg in args {
            all_args.push(expand(arg, &self.registry, &app_dir, &data, port));
        }
        let mut resolved_env = HashMap::new();
        for (key, value) in env {
            resolved_env.insert(
                key.clone(),
                expand(value, &self.registry, &app_dir, &data, port),
            );
        }
        let logs = install::logs_dir(install_dir);
        std::fs::create_dir_all(&logs).map_err(|e| format!("create app logs: {e}"))?;
        let log = logs.join("app.log");
        rotate_log(&log);
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| format!("open app log: {e}"))?;
        let _ = writeln!(
            log_file,
            "\n[{}] $ {} {} (cwd: {})",
            now_ms(),
            program.display(),
            all_args.join(" "),
            workdir.display()
        );
        let stdout = log_file
            .try_clone()
            .map_err(|e| format!("clone app log: {e}"))?;
        let stderr = log_file
            .try_clone()
            .map_err(|e| format!("clone app log: {e}"))?;
        let mut cmd = Command::new(&program);
        cmd.args(&all_args)
            .current_dir(&workdir)
            .envs(&resolved_env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
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
        let marker = claim_run_marker(app, install_dir, &self.registry, std::process::id())?;
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                remove_run_marker(marker.as_deref());
                return Err(format!("start {}: {err}", program.display()));
            }
        };
        let pid = child.id();
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
                break Err(format!("app exited during startup ({status})"));
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
            let mut states = self.states.lock().expect("app runtime lock");
            states.insert(
                app.id.clone(),
                RuntimeState {
                    status: AppStatus::Error,
                    port: None,
                    url: None,
                    error: Some(format!("{err}; log: {}", log.display())),
                    started_at: None,
                    kind: None,
                    run_marker: None,
                },
            );
            return Err(err);
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
                    kind: Some(RuntimeKind::Process { child, pid, log }),
                    run_marker: marker,
                },
            );
        }
        Ok(self.view(&app.id))
    }

    pub async fn start_static(
        &self,
        app: &RegisteredApp,
        install_dir: &Path,
    ) -> Result<RuntimeView, String> {
        let app_dir = install::app_dir(install_dir);
        let index = app_dir.join("index.html");
        if !index.is_file() {
            return Err(format!("static app is missing {}", index.display()));
        }
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
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("bind static app: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("static app addr: {e}"))?;
        let port = addr.port();
        let token = uuid::Uuid::new_v4().to_string();
        let cookie_name = format!("litecode_app_{port}");
        let spa = matches!(app.definition.launch, LaunchSpec::Static { spa: true });
        let auth = StaticAuth {
            cookie_name: cookie_name.clone(),
            token: token.clone(),
            used: Arc::new(AtomicBool::new(false)),
        };
        let app_router = if spa {
            Router::new()
                .fallback_service(ServeDir::new(&app_dir).fallback(ServeFile::new(index)))
                .layer(middleware::from_fn_with_state(auth, static_auth))
        } else {
            Router::new()
                .fallback_service(ServeDir::new(&app_dir))
                .layer(middleware::from_fn_with_state(auth, static_auth))
        };
        let marker = claim_run_marker(app, install_dir, &self.registry, std::process::id())?;
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

fn claim_run_marker(
    app: &RegisteredApp,
    install_dir: &Path,
    registry: &AppRegistry,
    pid: u32,
) -> Result<Option<PathBuf>, String> {
    let dir = install::run_dir(install_dir);
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
    // Every running app leaves a marker, not only single-instance ones. The
    // marker is what lets another workspace refuse an uninstall while this
    // app is still serving pages here.
    write_run_marker(&own, registry, pid)?;
    Ok(Some(own))
}

/// True while any workspace still has a live app process for this install.
/// Dead markers are cleaned here, so an uninstall after a crash can proceed.
pub fn has_live_run_marker(install_dir: &Path) -> bool {
    let dir = install::run_dir(install_dir);
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
    let authorized = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| {
            cookies.split(';').any(|part| {
                let part = part.trim();
                part == format!("{}={}", auth.cookie_name, auth.token)
            })
        })
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

fn allocate_port(spec: PortSpec) -> Result<u16, String> {
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

fn expand(value: &str, registry: &AppRegistry, app_dir: &Path, data: &Path, port: u16) -> String {
    value
        .replace(
            "{workspace}",
            &strip_verbatim(registry.workspace_root.clone()).to_string_lossy(),
        )
        .replace("{appdir}", &app_dir.to_string_lossy())
        .replace("{data}", &data.to_string_lossy())
        .replace("{port}", &port.to_string())
}

fn resolve_program(command: &str, app_dir: &Path) -> Result<(PathBuf, Vec<String>), String> {
    if command.eq_ignore_ascii_case("npm") {
        let (program, _) = crate::lsp::install::npm::npm_program_and_args(&[])
            .ok_or_else(|| "npm was not found on PATH".to_string())?;
        return Ok((program, Vec::new()));
    }
    if command.eq_ignore_ascii_case("npx") {
        let (program, _) = crate::lsp::install::npm::npm_program_and_args(&[])
            .ok_or_else(|| "npm was not found on PATH".to_string())?;
        return Ok((program, vec!["exec".into()]));
    }
    let path = Path::new(command);
    if path.is_absolute() {
        Ok((path.to_path_buf(), Vec::new()))
    } else if command.contains('/') || command.contains('\\') || command.ends_with(".exe") {
        Ok((app_dir.join(path), Vec::new()))
    } else {
        Ok((path.to_path_buf(), Vec::new()))
    }
}

fn rotate_log(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() > 5 * 1024 * 1024 {
        let backup = path.with_extension("log.1");
        let _ = std::fs::rename(path, backup);
    }
}
