//! Built-in app catalog. Versions and downloads are pinned here.

use std::collections::BTreeMap;

use super::schema::{
    AppCategory, AppDefinition, ArchiveFormat, BuiltinView, CheckSpec, DataScope, InstallKind,
    InstallSpec, InstallStep, LaunchSpec, Permission, PortSpec,
};

pub type BuiltinApp = (&'static str, AppDefinition);

fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn download(url: &str, sha256: Option<&str>, file: &str) -> InstallStep {
    InstallStep::Download {
        url: url.into(),
        sha256: sha256.map(str::to_string),
        file: file.into(),
    }
}

fn extract(file: &str, format: ArchiveFormat, strip: usize) -> InstallStep {
    InstallStep::Extract {
        file: file.into(),
        format,
        strip,
        into: ".".into(),
    }
}

fn run(command: &str, args: &[&str]) -> InstallStep {
    InstallStep::Run {
        command: command.into(),
        args: args.iter().map(|s| (*s).to_string()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: 600_000,
    }
}

fn steps(size_bytes: u64, steps: Vec<InstallStep>, check: &str) -> InstallSpec {
    InstallSpec {
        kind: InstallKind::Steps,
        installed: false,
        size_bytes: Some(size_bytes),
        steps,
        check: Some(CheckSpec { path: check.into() }),
    }
}

fn external(installed: bool) -> InstallSpec {
    InstallSpec {
        kind: InstallKind::None,
        installed,
        size_bytes: None,
        steps: Vec::new(),
        check: None,
    }
}

fn external_with_check(installed: bool, check: &str) -> InstallSpec {
    InstallSpec {
        kind: InstallKind::None,
        installed,
        size_bytes: None,
        steps: Vec::new(),
        check: Some(CheckSpec { path: check.into() }),
    }
}

fn process(
    command: &str,
    args: &[&str],
    cwd: Option<&str>,
    env: BTreeMap<String, String>,
    url: &str,
) -> LaunchSpec {
    LaunchSpec::Process {
        command: command.into(),
        args: args.iter().map(|s| (*s).to_string()).collect(),
        cwd: cwd.map(str::to_string),
        env,
        port: PortSpec::Auto("auto".into()),
        url: url.into(),
        ready_path: None,
        timeout_ms: 60_000,
    }
}

fn static_launch(spa: bool) -> LaunchSpec {
    LaunchSpec::Static { spa }
}

fn app(
    name: &str,
    description: &str,
    summary: &str,
    category: AppCategory,
    icon: &str,
    homepage: Option<&str>,
    readme_url: Option<&str>,
    install: InstallSpec,
    launch: LaunchSpec,
    data: DataScope,
    single_instance: bool,
    permissions: Vec<Permission>,
) -> AppDefinition {
    AppDefinition {
        name: name.into(),
        description: description.into(),
        description_i18n: BTreeMap::new(),
        category,
        icon: icon.into(),
        summary: summary.into(),
        homepage: homepage.map(str::to_string),
        readme_url: readme_url.map(str::to_string),
        install,
        launch,
        data,
        single_instance,
        permissions,
    }
}

/// NocoDB's package is a Nest application, not a CLI; this wrapper starts it
/// on loopback and moves its SQLite metadata into the workspace data directory.
const NOCODB_START: &str = r#"const express = require('express');
const { Noco } = require('nocodb');
const port = Number(process.env.PORT || 8080);
process.chdir(process.env.LITECODE_APP_DATA);
const server = express();
Noco.init({}, {}, server)
  .then(() => server.listen(port, '127.0.0.1'))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
"#;

/// All built-ins. Heavy apps are marked external on purpose: their fixed
/// directory is still the install target, but the user supplies the runtime.
pub fn builtins() -> Vec<BuiltinApp> {
    let mut out = Vec::new();

    out.push((
        "nocodb",
        app(
            "NocoDB",
            "Airtable-style database UI for local data.",
            "Node.js 22+ required. Installs the pinned nocodb npm package and starts it on loopback.",
            AppCategory::Data,
            "🗂️",
            Some("https://nocodb.com"),
            Some("https://raw.githubusercontent.com/nocodb/nocodb/develop/README.md"),
            steps(
                220 * 1024 * 1024,
                vec![run("npm", &["install", "--prefix", ".", "nocodb@0.301.3"])],
                "node_modules/nocodb/dist/bundle.js",
            ),
            process(
                "node",
                &["-e", NOCODB_START],
                Some("app"),
                BTreeMap::from([
                    ("PORT".into(), "{port}".into()),
                    ("LITECODE_APP_DATA".into(), "{data}".into()),
                    ("NC_DISABLE_TELE".into(), "true".into()),
                ]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    #[cfg(windows)]
    out.push((
        "openrefine",
        app(
            "OpenRefine",
            "Clean and transform tabular data locally.",
            "Windows portable build with Java bundled. Extracts into the fixed app directory.",
            AppCategory::Data,
            "🧹",
            Some("https://openrefine.org"),
            Some("https://raw.githubusercontent.com/OpenRefine/OpenRefine/master/README.md"),
            steps(
                192_670_582,
                vec![
                    download(
                        "https://github.com/OpenRefine/OpenRefine/releases/download/3.10.1/openrefine-win-with-java-3.10.1.zip",
                        Some("6155b20a8103244a26a8f21d41698ee52251538a838e3d5f2d5af507c8b6f3c8"),
                        "openrefine.zip",
                    ),
                    extract("openrefine.zip", ArchiveFormat::Zip, 1),
                ],
                "refine.bat",
            ),
            process(
                "cmd",
                &["/c", "refine.bat", "/p", "{port}", "/d", "{data}"],
                Some("app"),
                BTreeMap::from([
                    ("JAVA_HOME".into(), "{appdir}/server/target/jre".into()),
                ]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));
    #[cfg(not(windows))]
    out.push((
        "openrefine",
        app(
            "OpenRefine",
            "Clean and transform tabular data locally.",
            "Linux portable build with Java bundled. Extracts into the fixed app directory.",
            AppCategory::Data,
            "🧹",
            Some("https://openrefine.org"),
            Some("https://raw.githubusercontent.com/OpenRefine/OpenRefine/master/README.md"),
            steps(
                148_777_227,
                vec![
                    download(
                        "https://github.com/OpenRefine/OpenRefine/releases/download/3.10.1/openrefine-linux-3.10.1.tar.gz",
                        Some("806e38e415fa98c2dee400de575992514557a4e37e82c670b7cd2e34d667c4ba"),
                        "openrefine.tar.gz",
                    ),
                    extract("openrefine.tar.gz", ArchiveFormat::TarGz, 1),
                ],
                "refine",
            ),
            process(
                "./refine",
                &["-p", "{port}", "-d", "{data}"],
                Some("app"),
                empty_env(),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    #[cfg(windows)]
    out.push((
        "memos",
        app(
            "Memos",
            "A private, lightweight notes and microblog app.",
            "Windows release zip. Data stays in this workspace.",
            AppCategory::General,
            "📝",
            Some("https://www.usememos.com"),
            Some("https://raw.githubusercontent.com/usememos/memos/main/README.md"),
            steps(
                22_181_849,
                vec![
                    download(
                        "https://github.com/usememos/memos/releases/download/v0.31.0/memos_0.31.0_windows_amd64.zip",
                        Some("648974a02d4ffe1e038fcdd81562e8ef64c7c38bcee920f5e7750dd07a8d7723"),
                        "memos.zip",
                    ),
                    extract("memos.zip", ArchiveFormat::Zip, 0),
                ],
                "memos.exe",
            ),
            process(
                "memos.exe",
                &["--addr", "127.0.0.1", "--port", "{port}"],
                Some("app"),
                BTreeMap::from([("MEMOS_DATA".into(), "{data}".into())]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));
    #[cfg(not(windows))]
    out.push((
        "memos",
        app(
            "Memos",
            "A private, lightweight notes and microblog app.",
            "Linux release tarball. Data stays in this workspace.",
            AppCategory::General,
            "📝",
            Some("https://www.usememos.com"),
            Some("https://raw.githubusercontent.com/usememos/memos/main/README.md"),
            steps(
                21_825_067,
                vec![
                    download(
                        "https://github.com/usememos/memos/releases/download/v0.31.0/memos_0.31.0_linux_amd64.tar.gz",
                        Some("d99bf9de5e947cd41f7f1ae59e1e97d9af933d1bcc2d1316b3ab1ffe0a69e5c0"),
                        "memos.tar.gz",
                    ),
                    extract("memos.tar.gz", ArchiveFormat::TarGz, 0),
                ],
                "memos",
            ),
            process(
                "./memos",
                &["--addr", "127.0.0.1", "--port", "{port}"],
                Some("app"),
                BTreeMap::from([("MEMOS_DATA".into(), "{data}".into())]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    #[cfg(windows)]
    out.push((
        "vikunja",
        app(
            "Vikunja",
            "Self-hosted task and project management.",
            "Windows full release zip. The first launch creates its local database.",
            AppCategory::Task,
            "✅",
            Some("https://vikunja.io"),
            Some("https://raw.githubusercontent.com/go-vikunja/vikunja/main/README.md"),
            steps(
                38_988_483,
                vec![
                    download(
                        "https://github.com/go-vikunja/vikunja/releases/download/v2.7.1/vikunja-v2.7.1-windows-4.0-amd64.exe-full.zip",
                        Some("69f07c3b6515b621c347cc23791c6b4bbf71cca0a576e74487d74d22317c6718"),
                        "vikunja.zip",
                    ),
                    extract("vikunja.zip", ArchiveFormat::Zip, 0),
                ],
                "vikunja-v2.7.1-windows-4.0-amd64.exe",
            ),
            process(
                "vikunja-v2.7.1-windows-4.0-amd64.exe",
                &[],
                Some("app"),
                BTreeMap::from([
                    ("VIKUNJA_SERVICE_HTTPPORT".into(), "{port}".into()),
                    ("VIKUNJA_SERVICE_INTERFACE".into(), "127.0.0.1:{port}".into()),
                    ("VIKUNJA_SERVICE_PUBLICURL".into(), "http://127.0.0.1:{port}/".into()),
                    ("VIKUNJA_SERVICE_ROOTPATH".into(), "{data}".into()),
                    ("VIKUNJA_DATABASE_PATH".into(), "{data}/vikunja.db".into()),
                ]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));
    #[cfg(not(windows))]
    out.push((
        "vikunja",
        app(
            "Vikunja",
            "Self-hosted task and project management.",
            "External install: place the Linux binary in the fixed app directory or keep it on PATH.",
            AppCategory::Task,
            "✅",
            Some("https://vikunja.io"),
            Some("https://raw.githubusercontent.com/go-vikunja/vikunja/main/README.md"),
            external(true),
            process(
                "vikunja",
                &[],
                Some("workspace"),
                BTreeMap::from([
                    ("VIKUNJA_SERVICE_HTTPPORT".into(), "{port}".into()),
                    ("VIKUNJA_SERVICE_INTERFACE".into(), "127.0.0.1:{port}".into()),
                    ("VIKUNJA_SERVICE_PUBLICURL".into(), "http://127.0.0.1:{port}/".into()),
                    ("VIKUNJA_SERVICE_ROOTPATH".into(), "{data}".into()),
                    ("VIKUNJA_DATABASE_PATH".into(), "{data}/vikunja.db".into()),
                ]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    out.push((
        "audiomass",
        app(
            "AudioMass",
            "Full-featured waveform and multitrack audio editor.",
            "Static app from the AudioMass source archive. Microphone access is opt-in.",
            AppCategory::Audio,
            "🎚️",
            Some("https://audiomass.co"),
            Some("https://raw.githubusercontent.com/pkalogiros/audiomass/master/README.md"),
            steps(
                20 * 1024 * 1024,
                vec![
                    download(
                        "https://github.com/pkalogiros/audiomass/archive/21f5ee1362a47be6f0dbe6e4969a15e43d21b044.zip",
                        Some("c35845b82d2daa9c708d3e04d49705b784ee7211220848f668dcd2a22231e0d5"),
                        "audiomass.zip",
                    ),
                    extract("audiomass.zip", ArchiveFormat::Zip, 2),
                ],
                "index.html",
            ),
            static_launch(false),
            DataScope::Workspace,
            false,
            vec![Permission::Microphone],
        ),
    ));

    out.push((
        "marimo",
        app(
            "marimo",
            "Reactive Python notebooks in the browser.",
            "Creates a local virtualenv and installs pinned marimo. Python 3.10+ required.",
            AppCategory::Analysis,
            "🐍",
            Some("https://marimo.io"),
            Some("https://raw.githubusercontent.com/marimo-team/marimo/main/README.md"),
            steps(
                60 * 1024 * 1024,
                vec![
                    run("python", &["-m", "venv", ".venv"]),
                    run(
                        if cfg!(windows) {
                            ".venv/Scripts/python.exe"
                        } else {
                            ".venv/bin/python"
                        },
                        &[
                            "-m",
                            "pip",
                            "install",
                            "--disable-pip-version-check",
                            "marimo==0.25.2",
                        ],
                    ),
                ],
                if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                },
            ),
            process(
                if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                },
                &[
                    "-m",
                    "marimo",
                    "edit",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    "{port}",
                    "--headless",
                    "--no-token",
                ],
                Some("workspace"),
                empty_env(),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    out.push((
        "quarto",
        app(
            "Quarto",
            "Scientific and technical publishing from Markdown.",
            "External install: Quarto must be available on PATH. Preview runs in this workspace.",
            AppCategory::Document,
            "📄",
            Some("https://quarto.org"),
            Some("https://raw.githubusercontent.com/quarto-dev/quarto-cli/main/README.md"),
            external(true),
            process(
                "quarto",
                &["preview", "--port", "{port}", "--no-browser"],
                Some("workspace"),
                empty_env(),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    out.push((
        "remotion",
        app(
            "Remotion",
            "Create videos with React in the current workspace.",
            "External install: run the Remotion CLI against the current workspace project. Remotion Studio must listen on loopback; a LAN-exposed Studio is refused.",
            AppCategory::Video,
            "🎬",
            Some("https://www.remotion.dev"),
            Some("https://raw.githubusercontent.com/remotion-dev/remotion/main/README.md"),
            external(true),
            process(
                "npx",
                &["remotion", "studio", "--port", "{port}"],
                Some("workspace"),
                empty_env(),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    out.push((
        "comfyui",
        app(
            "ComfyUI",
            "Node-based Stable Diffusion workflow editor.",
            "External install: put the ComfyUI portable runtime in the fixed app directory. Models are not bundled.",
            AppCategory::Image,
            "🖼️",
            Some("https://www.comfy.org"),
            Some("https://raw.githubusercontent.com/comfyanonymous/ComfyUI/master/README.md"),
            external_with_check(
                true,
                if cfg!(windows) { "ComfyUI/main.py" } else { "main.py" },
            ),
            process(
                if cfg!(windows) {
                    "python_embeded/python.exe"
                } else {
                    "python"
                },
                if cfg!(windows) {
                    &["ComfyUI/main.py", "--port", "{port}", "--listen", "127.0.0.1"]
                } else {
                    &["main.py", "--port", "{port}", "--listen", "127.0.0.1"]
                },
                Some("app"),
                BTreeMap::from([("COMFYUI_DATA".into(), "{data}".into())]),
                "http://127.0.0.1:{port}/",
            ),
            DataScope::App,
            true,
            vec![],
        ),
    ));

    out.push((
        "excalidraw",
        app(
            "Excalidraw",
            "Hand-drawn style whiteboard and diagrams.",
            "External static build: place the built app in the fixed app directory.",
            AppCategory::Design,
            "✏️",
            Some("https://excalidraw.com"),
            Some("https://raw.githubusercontent.com/excalidraw/excalidraw/master/README.md"),
            external_with_check(true, "index.html"),
            static_launch(false),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    out.push((
        "rawgraphs",
        app(
            "RAWGraphs",
            "Open-source visualization framework for tabular data.",
            "External static build: place the built app in the fixed app directory.",
            AppCategory::Analysis,
            "📊",
            Some("https://rawgraphs.io"),
            Some("https://raw.githubusercontent.com/rawgraphs/rawgraphs-app/master/README.md"),
            external_with_check(true, "index.html"),
            static_launch(false),
            DataScope::Workspace,
            false,
            vec![],
        ),
    ));

    // The media browser is a panel LiteCode paints: no install, no process,
    // and no loopback server, so it must never be started or stopped.
    out.push((
        "gallery",
        app(
            "Gallery",
            "Browse this workspace's images, video, and audio.",
            "Built-in media browser. Scans the workspace, filters by folder or type, and opens files in the system app.",
            AppCategory::Image,
            "🖼️",
            None,
            None,
            InstallSpec {
                kind: InstallKind::None,
                installed: true,
                size_bytes: None,
                steps: Vec::new(),
                check: None,
            },
            LaunchSpec::Builtin {
                view: BuiltinView::Gallery,
            },
            DataScope::Workspace,
            true,
            vec![],
        ),
    ));

    // The required `description` is the English fallback. Built-ins also ship a
    // zh-CN one-liner so the same sentence reads naturally in both UI languages.
    for (id, text) in [
        ("nocodb", "Airtable 风格的本地数据库界面。"),
        ("openrefine", "在本地清洗和转换表格数据。"),
        ("memos", "私密、轻量的笔记与微播客应用。"),
        ("vikunja", "自托管的任务与项目管理。"),
        ("audiomass", "功能完整的波形与多轨音频编辑器。"),
        ("marimo", "在浏览器里运行响应式 Python 笔记本。"),
        ("quarto", "从 Markdown 发布科技与学术内容。"),
        ("remotion", "在当前工作区用 React 制作视频。"),
        ("comfyui", "节点式 Stable Diffusion 工作流编辑器。"),
        ("excalidraw", "手绘风格的白板与示意图。"),
        ("rawgraphs", "面向表格数据的开源可视化框架。"),
        ("gallery", "浏览工作区里的图片、视频和音频。"),
    ] {
        for entry in out.iter_mut() {
            if entry.0 == id {
                entry
                    .1
                    .description_i18n
                    .insert("zh-CN".into(), text.to_string());
            }
        }
    }

    out
}
