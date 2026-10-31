//! Built-in app catalog. Only apps LiteCode can fully install, uninstall, and
//! keep in a managed directory live here. Everything else is a recommendation.

use std::collections::BTreeMap;

use super::builtin::{
    ArchiveFormat, BuiltinApp, BuiltinDataScope, BuiltinDocs, CheckSpec, InstallSpec, InstallStep,
};
use super::schema::{AppCategory, AppDefinition, BuiltinView, LaunchSpec, Permission, PortSpec};

fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn download(url: &str, sha256: &str, file: &str) -> InstallStep {
    InstallStep::Download {
        url: url.into(),
        sha256: Some(sha256.into()),
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

fn write(file: &str, contents: &str) -> InstallStep {
    InstallStep::Write {
        file: file.into(),
        contents: contents.into(),
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

#[cfg(windows)]
fn run_with_env(command: &str, args: &[&str], env: BTreeMap<String, String>) -> InstallStep {
    InstallStep::Run {
        command: command.into(),
        args: args.iter().map(|s| (*s).to_string()).collect(),
        cwd: None,
        env,
        timeout_ms: 600_000,
    }
}

/// A managed install: pinned steps plus the file that proves they worked.
fn steps(steps: Vec<InstallStep>, check: &str) -> Option<InstallSpec> {
    Some(InstallSpec {
        steps,
        check: Some(CheckSpec { path: check.into() }),
    })
}

fn process(
    command: &str,
    args: &[&str],
    cwd: &str,
    env: BTreeMap<String, String>,
    url: &str,
) -> LaunchSpec {
    LaunchSpec::Process {
        command: command.into(),
        args: args.iter().map(|s| (*s).to_string()).collect(),
        cwd: cwd.into(),
        env,
        port: PortSpec::Auto("auto".into()),
        url: url.into(),
        ready_path: None,
        timeout_ms: 60_000,
    }
}

fn static_launch() -> LaunchSpec {
    LaunchSpec::Static {
        dir: "{appdir}".into(),
        spa: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn app(
    id: &'static str,
    name: &str,
    description: &str,
    category: AppCategory,
    icon: &str,
    repo: Option<&str>,
    docs_url: Option<&str>,
    docs: BuiltinDocs,
    install: Option<InstallSpec>,
    launch: LaunchSpec,
    data: BuiltinDataScope,
    single_instance: bool,
    permissions: Vec<Permission>,
) -> BuiltinApp {
    BuiltinApp {
        id,
        definition: AppDefinition {
            name: name.into(),
            description: description.into(),
            description_i18n: BTreeMap::new(),
            category,
            icon: icon.into(),
            repo: repo.map(Into::into),
            docs: docs_url.map(Into::into),
            launch,
            single_instance,
            permissions,
        },
        install,
        data,
        docs,
    }
}

const NOCODB_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/nocodb.md"),
    zh: include_str!("docs/nocodb.zh-CN.md"),
};
const OPENREFINE_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/openrefine.md"),
    zh: include_str!("docs/openrefine.zh-CN.md"),
};
const MEMOS_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/memos.md"),
    zh: include_str!("docs/memos.zh-CN.md"),
};
#[cfg(windows)]
const VIKUNJA_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/vikunja.md"),
    zh: include_str!("docs/vikunja.zh-CN.md"),
};
const AUDIOMASS_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/audiomass.md"),
    zh: include_str!("docs/audiomass.zh-CN.md"),
};
const XPRITE_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/xprite.md"),
    zh: include_str!("docs/xprite.zh-CN.md"),
};
const MARIMO_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/marimo.md"),
    zh: include_str!("docs/marimo.zh-CN.md"),
};
const GALLERY_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/gallery.md"),
    zh: include_str!("docs/gallery.zh-CN.md"),
};
#[cfg(windows)]
const SQLPAGE_DOCS: BuiltinDocs = BuiltinDocs {
    en: include_str!("docs/sqlpage.md"),
    zh: include_str!("docs/sqlpage.zh-CN.md"),
};

/// NocoDB's package is a Nest application, not a CLI; this wrapper starts it
/// on loopback and moves its SQLite metadata into the workspace data directory.
const NOCODB_START: &str = r#"const express = require('express');
const { Noco } = require('nocodb');
const port = Number(process.env.PORT || 8080);
process.chdir(process.env.LITECODE_APP_DATA);
const server = express();
const httpServer = server.listen(port, '127.0.0.1', () => {
  Noco.init({}, httpServer, server)
    .then((app) => server.use(app))
    .catch((error) => {
      console.error(error);
      process.exit(1);
    });
});
httpServer.on('error', (error) => {
  console.error(error);
  process.exit(1);
});
"#;

#[cfg(windows)]
const SQLPAGE_BOOTSTRAP: &str = r#"$site = Join-Path $env:LITECODE_APP_DATA 'site'
New-Item -ItemType Directory -Force -Path $site | Out-Null
$index = Join-Path $site 'index.sql'
if (-not (Test-Path -LiteralPath $index)) {
  $text = @'
SELECT 'text' AS component, 'SQLPage is ready. Edit site/index.sql.' AS contents;
'@
  [IO.File]::WriteAllText($index, $text, (New-Object System.Text.UTF8Encoding -ArgumentList $false))
}
"#;

/// All built-ins. Each one has a managed install, so LiteCode owns its app
/// directory and can remove it again. Apps that need a user-supplied runtime
/// are recommendations, not built-ins.
pub fn builtins() -> Vec<BuiltinApp> {
    let mut out = vec![nocodb(), openrefine(), memos()];
    #[cfg(windows)]
    out.push(vikunja());
    #[cfg(windows)]
    out.push(sqlpage());
    out.push(audiomass());
    out.push(xprite());
    out.push(marimo());
    out.push(gallery());
    localize(&mut out);
    out
}

fn nocodb() -> BuiltinApp {
    app(
        "nocodb",
        "NocoDB",
        "Airtable-style database UI for local data.",
        AppCategory::Data,
        "🗂️",
        Some("https://github.com/nocodb/nocodb"),
        Some("https://docs.nocodb.com/"),
        NOCODB_DOCS,
        steps(
            vec![run("npm", &["install", "--prefix", ".", "nocodb@0.301.3"])],
            "node_modules/nocodb/dist/bundle.js",
        ),
        process(
            "node",
            &["-e", NOCODB_START],
            "{appdir}",
            BTreeMap::from([
                ("PORT".into(), "{port}".into()),
                ("LITECODE_APP_DATA".into(), "{data}".into()),
                ("NC_DISABLE_TELE".into(), "true".into()),
            ]),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

#[cfg(windows)]
fn openrefine() -> BuiltinApp {
    app(
        "openrefine",
        "OpenRefine",
        "Clean and transform tabular data locally.",
        AppCategory::Data,
        "🧹",
        Some("https://github.com/OpenRefine/OpenRefine"),
        Some("https://openrefine.org/docs"),
        OPENREFINE_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/OpenRefine/OpenRefine/releases/download/3.10.1/openrefine-win-with-java-3.10.1.zip",
                    "6155b20a8103244a26a8f21d41698ee52251538a838e3d5f2d5af507c8b6f3c8",
                    "openrefine.zip",
                ),
                extract("openrefine.zip", ArchiveFormat::Zip, 1),
            ],
            "refine.bat",
        ),
        process(
            "cmd",
            &["/c", "refine.bat", "/p", "{port}", "/d", "{data}"],
            "{appdir}",
            BTreeMap::from([("JAVA_HOME".into(), "{appdir}/server/target/jre".into())]),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

#[cfg(not(windows))]
fn openrefine() -> BuiltinApp {
    app(
        "openrefine",
        "OpenRefine",
        "Clean and transform tabular data locally.",
        AppCategory::Data,
        "🧹",
        Some("https://github.com/OpenRefine/OpenRefine"),
        Some("https://openrefine.org/docs"),
        OPENREFINE_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/OpenRefine/OpenRefine/releases/download/3.10.1/openrefine-linux-3.10.1.tar.gz",
                    "806e38e415fa98c2dee400de575992514557a4e37e82c670b7cd2e34d667c4ba",
                    "openrefine.tar.gz",
                ),
                extract("openrefine.tar.gz", ArchiveFormat::TarGz, 1),
            ],
            "refine",
        ),
        process(
            "./refine",
            &["-p", "{port}", "-d", "{data}"],
            "{appdir}",
            empty_env(),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

#[cfg(windows)]
fn memos() -> BuiltinApp {
    app(
        "memos",
        "Memos",
        "A private, lightweight notes and microblog app.",
        AppCategory::General,
        "📝",
        Some("https://github.com/usememos/memos"),
        Some("https://www.usememos.com/docs"),
        MEMOS_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/usememos/memos/releases/download/v0.31.0/memos_0.31.0_windows_amd64.zip",
                    "648974a02d4ffe1e038fcdd81562e8ef64c7c38bcee920f5e7750dd07a8d7723",
                    "memos.zip",
                ),
                extract("memos.zip", ArchiveFormat::Zip, 0),
            ],
            "memos.exe",
        ),
        process(
            "memos.exe",
            &["--addr", "127.0.0.1", "--port", "{port}"],
            "{appdir}",
            BTreeMap::from([("MEMOS_DATA".into(), "{data}".into())]),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

#[cfg(not(windows))]
fn memos() -> BuiltinApp {
    app(
        "memos",
        "Memos",
        "A private, lightweight notes and microblog app.",
        AppCategory::General,
        "📝",
        Some("https://github.com/usememos/memos"),
        Some("https://www.usememos.com/docs"),
        MEMOS_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/usememos/memos/releases/download/v0.31.0/memos_0.31.0_linux_amd64.tar.gz",
                    "d99bf9de5e947cd41f7f1ae59e1e97d9af933d1bcc2d1316b3ab1ffe0a69e5c0",
                    "memos.tar.gz",
                ),
                extract("memos.tar.gz", ArchiveFormat::TarGz, 0),
            ],
            "memos",
        ),
        process(
            "./memos",
            &["--addr", "127.0.0.1", "--port", "{port}"],
            "{appdir}",
            BTreeMap::from([("MEMOS_DATA".into(), "{data}".into())]),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

fn vikunja_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        // Vikunja 2.x folded the port into service.interface; service.httpPort is gone.
        (
            "VIKUNJA_SERVICE_INTERFACE".into(),
            "127.0.0.1:{port}".into(),
        ),
        (
            "VIKUNJA_SERVICE_PUBLICURL".into(),
            "http://127.0.0.1:{port}/".into(),
        ),
        ("VIKUNJA_SERVICE_ROOTPATH".into(), "{data}".into()),
        ("VIKUNJA_DATABASE_PATH".into(), "{data}/vikunja.db".into()),
    ])
}

#[cfg(windows)]
fn vikunja() -> BuiltinApp {
    app(
        "vikunja",
        "Vikunja",
        "Self-hosted task and project management.",
        AppCategory::Task,
        "✅",
        Some("https://github.com/go-vikunja/vikunja"),
        Some("https://vikunja.io/docs/"),
        VIKUNJA_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/go-vikunja/vikunja/releases/download/v2.7.1/vikunja-v2.7.1-windows-4.0-amd64.exe-full.zip",
                    "69f07c3b6515b621c347cc23791c6b4bbf71cca0a576e74487d74d22317c6718",
                    "vikunja.zip",
                ),
                extract("vikunja.zip", ArchiveFormat::Zip, 0),
            ],
            "vikunja-v2.7.1-windows-4.0-amd64.exe",
        ),
        process(
            "vikunja-v2.7.1-windows-4.0-amd64.exe",
            &[],
            "{appdir}",
            vikunja_env(),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

#[cfg(windows)]
fn sqlpage() -> BuiltinApp {
    app(
        "sqlpage",
        "SQLPage",
        "Build data apps from SQL files locally.",
        AppCategory::Data,
        "📊",
        Some("https://github.com/sqlpage/SQLPage"),
        Some("https://sql-page.com/"),
        SQLPAGE_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/sqlpage/SQLPage/releases/download/v0.47.0/sqlpage-windows.zip",
                    "ea01ff6f8bdfcb8150811542f6afdb587b20a4febf510b550539f6cb384279a5",
                    "sqlpage.zip",
                ),
                extract("sqlpage.zip", ArchiveFormat::Zip, 0),
                run_with_env(
                    "powershell",
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        SQLPAGE_BOOTSTRAP,
                    ],
                    BTreeMap::from([("LITECODE_APP_DATA".into(), "{data}".into())]),
                ),
            ],
            "sqlpage.exe",
        ),
        process(
            "{appdir}/sqlpage.exe",
            &[],
            "{appdir}",
            BTreeMap::from([
                ("LISTEN_ON".into(), "127.0.0.1:{port}".into()),
                (
                    "SQLPAGE_CONFIGURATION_DIRECTORY".into(),
                    "{appdir}/sqlpage".into(),
                ),
                ("SQLPAGE_WEB_ROOT".into(), "{data}/site".into()),
                (
                    "DATABASE_URL".into(),
                    "sqlite://{data}/sqlpage.db?mode=rwc".into(),
                ),
            ]),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

fn audiomass() -> BuiltinApp {
    app(
        "audiomass",
        "AudioMass",
        "Full-featured waveform and multitrack audio editor.",
        AppCategory::Audio,
        "🎚️",
        Some("https://github.com/pkalogiros/audiomass"),
        None,
        AUDIOMASS_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/pkalogiros/audiomass/archive/21f5ee1362a47be6f0dbe6e4969a15e43d21b044.zip",
                    "c35845b82d2daa9c708d3e04d49705b784ee7211220848f668dcd2a22231e0d5",
                    "audiomass.zip",
                ),
                extract("audiomass.zip", ArchiveFormat::Zip, 2),
            ],
            "index.html",
        ),
        static_launch(),
        BuiltinDataScope::Workspace,
        false,
        vec![Permission::Microphone],
    )
}

/// Xprite's release ships the hashed web assets only, so the entry page comes
/// from the pinned production page; the two are released together.
const XPRITE_INDEX: &str = include_str!("assets/xprite/index.html");

fn xprite() -> BuiltinApp {
    app(
        "xprite",
        "Xprite",
        "Pixel art and sprite animation in the browser. Opens .ase/.aseprite.",
        AppCategory::Image,
        "🎨",
        Some("https://github.com/rhinoc/xprite"),
        Some("https://xprite.cc/help/"),
        XPRITE_DOCS,
        steps(
            vec![
                download(
                    "https://github.com/rhinoc/xprite/releases/download/v0.1.22/xprite-hashed-assets-v0.1.22.tar.gz",
                    "569e5d3d2e5e6cd0d282c8da1e85574ec22bfa8cbfa014e780a03c4ba3f90b5d",
                    "xprite.tar.gz",
                ),
                extract("xprite.tar.gz", ArchiveFormat::TarGz, 1),
                write("index.html", XPRITE_INDEX),
            ],
            "index.html",
        ),
        static_launch(),
        BuiltinDataScope::Workspace,
        false,
        vec![
            Permission::ClipboardRead,
            Permission::ClipboardWrite,
            Permission::Fullscreen,
        ],
    )
}

fn marimo_python() -> &'static str {
    if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    }
}

/// Launch runs from the workspace, so the interpreter is addressed explicitly.
fn marimo_launch_python() -> &'static str {
    if cfg!(windows) {
        "{appdir}/.venv/Scripts/python.exe"
    } else {
        "{appdir}/.venv/bin/python"
    }
}

fn marimo() -> BuiltinApp {
    app(
        "marimo",
        "marimo",
        "Reactive Python notebooks in the browser.",
        AppCategory::Analysis,
        "🐍",
        Some("https://github.com/marimo-team/marimo"),
        Some("https://docs.marimo.io/"),
        MARIMO_DOCS,
        steps(
            vec![
                run("python", &["-m", "venv", ".venv"]),
                run(
                    marimo_python(),
                    &[
                        "-m",
                        "pip",
                        "install",
                        "--disable-pip-version-check",
                        "marimo==0.25.2",
                    ],
                ),
            ],
            marimo_python(),
        ),
        process(
            marimo_launch_python(),
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
            "{workspace}",
            empty_env(),
            "http://127.0.0.1:{port}/",
        ),
        BuiltinDataScope::Workspace,
        false,
        vec![],
    )
}

/// The media browser is a panel LiteCode paints: no install, no process, and no
/// loopback server, so it must never be started or stopped.
fn gallery() -> BuiltinApp {
    app(
        "gallery",
        "Gallery",
        "Browse this workspace's images, video, and audio.",
        AppCategory::Image,
        "🖼️",
        None,
        None,
        GALLERY_DOCS,
        None,
        LaunchSpec::Builtin {
            view: BuiltinView::Gallery,
        },
        BuiltinDataScope::Workspace,
        true,
        vec![],
    )
}

/// The required description is the English fallback. Built-ins also ship a
/// zh-CN one-liner so the same sentence reads naturally in both UI languages.
pub fn localize(builtins: &mut [BuiltinApp]) {
    for (id, text) in [
        ("nocodb", "Airtable 风格的本地数据库界面。"),
        ("openrefine", "在本地清洗和转换表格数据。"),
        ("memos", "私密、轻量的笔记与微博客应用。"),
        ("vikunja", "自托管的任务与项目管理。"),
        ("sqlpage", "用 SQL 文件在本地搭建数据应用。"),
        ("audiomass", "功能完整的波形与多轨音频编辑器。"),
        ("xprite", "浏览器里的像素画与精灵动画编辑器。"),
        ("marimo", "在浏览器里运行响应式 Python 笔记本。"),
        ("gallery", "浏览工作区里的图片、视频和音频。"),
    ] {
        if let Some(entry) = builtins.iter_mut().find(|app| app.id == id) {
            entry
                .definition
                .description_i18n
                .insert("zh-CN".into(), text.to_string());
        }
    }
}
