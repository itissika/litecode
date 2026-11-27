//! The scored surface is one file; these tests keep that file honest and keep the
//! applier from quietly binding less than it says.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use litecode::config::eval_profile::{self, AGENT_ID, Applied, EvalProfile, RUN_CODE_ID, WebMode};
use litecode::config::global_db::{self, EvalSurfaceOverride, store};
use litecode::config::resolved::{ResolvedConfig, WorkspaceState};
use litecode::config::schema::{AgentToolBinding, GlobalSettings, ToolPreset};
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard};
use litecode::tool::availability::available_tools;
use litecode::tool::capability::capability_catalog;
use litecode::types::Result;
use rusqlite::OptionalExtension;

fn repo_profile_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/profile.toml")
}

fn repo_profile() -> EvalProfile {
    EvalProfile::from_path(&repo_profile_path()).expect("the repository profile parses")
}

/// Every id the registry can put in front of the model: the members of every
/// capability card, plus the code transport. A tool that is added to the
/// registry shows up here on its own.
fn registry_ids() -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let resolved = ConfigManager::resolve_without_catalog(
        GlobalSettings::default(),
        WorkspaceState::new(dir.path()),
    );
    let mut ids: BTreeSet<String> = available_tools(&resolved)
        .into_iter()
        .flat_map(|tool| tool.members)
        .collect();
    ids.insert(RUN_CODE_ID.to_string());
    ids
}

/// The override the applier must store for one `Applied`: every bound card with
/// the tier it recorded, and no card the profile left out.
fn expected_surface(applied: &Applied) -> EvalSurfaceOverride {
    EvalSurfaceOverride {
        agent_id: AGENT_ID.to_string(),
        cards: applied
            .cards
            .iter()
            .map(|card| {
                let preset = match applied.permissions.get(card).map(String::as_str) {
                    Some("all") => Some(ToolPreset::All),
                    Some("safe") => Some(ToolPreset::Safe),
                    Some(other) => panic!("unknown tier {other}"),
                    None => None,
                };
                (card.clone(), preset)
            })
            .collect(),
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("litecode.db");
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).expect("workspace dir");
        ConfigManager::load_global_from(&db).expect("a fresh db seeds itself");
        Self {
            _dir: dir,
            db,
            workspace,
        }
    }

    fn settings(&self) -> GlobalSettings {
        ConfigManager::load_global_from(&self.db).expect("load settings")
    }

    fn resolved(&self) -> ResolvedConfig {
        ConfigManager::resolve_without_catalog(
            self.settings(),
            WorkspaceState::new(&self.workspace),
        )
    }

    fn apply(&self, profile: &EvalProfile, web: WebMode) -> Result<Applied> {
        let writer = SettingsWriter::with_path(&self.db, Arc::new(TurnGuard::new()));
        eval_profile::apply(&writer, &self.resolved(), profile, Some(web))
    }

    fn binding_rows(&self) -> i64 {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        conn.query_row(
            "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1",
            [AGENT_ID],
            |row| row.get(0),
        )
        .expect("count bindings")
    }

    fn agent_tool_row(&self, tool_id: &str) -> Option<bool> {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        conn.query_row(
            "SELECT enabled FROM agent_tools WHERE agent_id = ?1 AND tool_id = ?2",
            [AGENT_ID, tool_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )
        .optional()
        .expect("read one binding row")
    }

    /// The raw `meta.eval_surface` text: `None` means no override is in force.
    fn eval_surface_raw(&self) -> Option<String> {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [global_db::EVAL_SURFACE_META_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .expect("read meta.eval_surface")
    }

    /// The override as the loader sees it.
    fn eval_surface(&self) -> Option<EvalSurfaceOverride> {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        store::load_eval_surface(&conn).expect("parse the stored override")
    }

    fn eval_surface_rows(&self) -> i64 {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        conn.query_row(
            "SELECT COUNT(*) FROM meta WHERE key = ?1",
            [global_db::EVAL_SURFACE_META_KEY],
            |row| row.get(0),
        )
        .expect("count override rows")
    }

    fn default_tools(&self) -> std::collections::HashMap<String, AgentToolBinding> {
        self.settings()
            .agents
            .get(AGENT_ID)
            .map(|agent| agent.tools.clone())
            .expect("the default agent is always present")
    }
}

/// The drift gate: a tool the registry can bind must be classified, exactly once.
#[test]
fn the_repository_profile_classifies_every_registry_tool() {
    let profile = repo_profile();
    let classified: BTreeSet<String> = profile
        .tools
        .on
        .iter()
        .chain(&profile.tools.off)
        .chain(&profile.tools.network)
        .cloned()
        .collect();
    let registry = registry_ids();
    let missing: Vec<&String> = registry.difference(&classified).collect();
    let unknown: Vec<&String> = classified.difference(&registry).collect();
    assert!(
        missing.is_empty(),
        "the registry binds tools the profile does not classify: {missing:?}"
    );
    assert!(
        unknown.is_empty(),
        "the profile names tools the registry does not bind: {unknown:?}"
    );
}

/// Acceptance 1: after applying, the model-visible surface is exactly `tools.on`
/// (`run_code` excepted: it is the transport, not a binding).
#[test]
fn applying_binds_exactly_the_profile_surface() {
    let fixture = Fixture::new();
    let profile = repo_profile();
    let applied = fixture.apply(&profile, WebMode::Off).expect("apply");
    assert_eq!(
        fixture.eval_surface(),
        Some(expected_surface(&applied)),
        "the scored surface travels through meta.eval_surface"
    );

    let settings = fixture.settings();
    let bound = settings.agents.get(AGENT_ID).expect("default agent");
    let catalog = capability_catalog(&fixture.resolved());
    let visible: BTreeSet<String> = catalog
        .selected_for_profile(bound, 0)
        .iter()
        .flat_map(|capability| capability.member_ids())
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<String> = profile
        .tools
        .on
        .iter()
        .filter(|id| *id != RUN_CODE_ID)
        .cloned()
        .collect();
    assert_eq!(visible, expected, "{applied:?}");
    assert!(
        profile.tools.on.iter().any(|id| id == RUN_CODE_ID),
        "the code transport is stated in tools.on"
    );
    for id in &profile.tools.off {
        assert!(!visible.contains(id), "'{id}' is off but bound");
        assert!(
            bound.tools.get(id).is_none_or(|binding| !binding.enabled),
            "'{id}' is still switched on"
        );
    }
    for id in &profile.tools.network {
        assert!(!visible.contains(id), "'{id}' is web and web is off here");
    }
}

/// Acceptance 1, web side: the same file with the probe saying `on` adds the
/// network tools and nothing else.
#[test]
fn a_networked_container_gets_the_web_group() {
    let fixture = Fixture::new();
    let profile = repo_profile();
    fixture.apply(&profile, WebMode::On).expect("apply");
    let settings = fixture.settings();
    let bound = settings.agents.get(AGENT_ID).expect("default agent");
    for id in &profile.tools.network {
        assert!(
            bound.tools.contains_key(id),
            "'{id}' must be bound when web is on"
        );
    }
    let catalog = capability_catalog(&fixture.resolved());
    let visible: BTreeSet<String> = catalog
        .selected_for_profile(bound, 0)
        .iter()
        .flat_map(|capability| capability.member_ids())
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<String> = profile
        .tools
        .on
        .iter()
        .chain(&profile.tools.network)
        .filter(|id| *id != RUN_CODE_ID)
        .cloned()
        .collect();
    assert_eq!(visible, expected);
}

/// Acceptance 3: applying twice leaves the same surface behind — one `meta`
/// row, and the parsed override still exactly the profile's surface.
#[test]
fn applying_twice_leaves_one_row_per_tool() {
    let fixture = Fixture::new();
    let profile = repo_profile();
    let first = fixture.apply(&profile, WebMode::Off).expect("first apply");
    let surface_after_first = fixture
        .eval_surface()
        .expect("the first apply stores an override");
    assert_eq!(
        surface_after_first,
        expected_surface(&first),
        "the stored override is exactly the applied surface"
    );
    let tools_after_first = fixture.default_tools();

    let second = fixture.apply(&profile, WebMode::Off).expect("second apply");
    assert_eq!(first, second, "the same file must apply to the same result");
    assert_eq!(
        fixture.eval_surface(),
        Some(surface_after_first),
        "the second apply must not change the override"
    );
    assert_eq!(
        fixture.eval_surface_rows(),
        1,
        "an override is one meta row, not an accumulating list"
    );
    let raw = fixture.eval_surface_raw().expect("the override is one meta row");
    assert!(
        raw.contains("\"agent_id\":\"default\""),
        "the row records which agent the surface is for: {raw}"
    );
    assert_eq!(
        fixture.binding_rows(),
        0,
        "a profile writes no user bindings: the surface lives in meta"
    );
    assert_eq!(
        fixture.default_tools(),
        tools_after_first,
        "the second apply must not change the resolved surface"
    );
}

/// Acceptance 5: the shell tier is All in the override the runtime reads, and no
/// user binding row pretends to hold it.
#[test]
fn the_shell_tier_is_all() {
    let fixture = Fixture::new();
    fixture.apply(&repo_profile(), WebMode::Off).expect("apply");
    let surface = fixture.eval_surface().expect("the override is stored");
    assert_eq!(surface.cards.get("exec"), Some(&Some(ToolPreset::All)));
    assert_eq!(
        fixture.default_tools()["exec"].preset,
        Some(ToolPreset::All),
        "the resolved binding carries the tier"
    );
    assert_eq!(
        fixture.agent_tool_row("exec"),
        None,
        "the tier lives in the override, not in a user binding row"
    );
}

/// Acceptance 2: an id the registry does not have is refused and named.
#[test]
fn an_unknown_id_is_refused_and_named() {
    let fixture = Fixture::new();
    let profile = EvalProfile::parse(
        r#"name = "t"
[tools]
on = ["read", "reed"]
[permissions]
read = "all"
"#,
    )
    .expect("shape is fine; the registry is what refuses it");
    let error = fixture
        .apply(&profile, WebMode::Off)
        .expect_err("'reed' is not a tool");
    let message = error.to_string();
    assert!(message.contains("'reed'"), "{message}");
    assert!(message.contains("unknown tool"), "{message}");
    assert!(
        message.contains(", read,"),
        "the error lists what may be used instead: {message}"
    );
}

/// A card is bound as a whole, so half of it is a file that does not say what it
/// means.
#[test]
fn half_a_card_is_refused() {
    let fixture = Fixture::new();
    let profile = EvalProfile::parse(
        r#"name = "t"
[tools]
on = ["memory.list"]
"#,
    )
    .expect("parses");
    let error = fixture
        .apply(&profile, WebMode::Off)
        .expect_err("memory is a card of three");
    let message = error.to_string();
    assert!(message.contains("half bound"), "{message}");
    assert!(message.contains("memory.upsert"), "{message}");
}

/// A bound dial without a stated tier would apply an inherited default.
#[test]
fn a_bound_tool_without_a_tier_is_refused() {
    let fixture = Fixture::new();
    let profile = EvalProfile::parse(
        r#"name = "t"
[tools]
on = ["read", "run_code"]
"#,
    )
    .expect("parses");
    let error = fixture
        .apply(&profile, WebMode::Off)
        .expect_err("read has a dial");
    let message = error.to_string();
    assert!(message.contains("'read'"), "{message}");
    assert!(message.contains("permissions"), "{message}");
}

/// `web = "auto"` is only resolvable with the probe's answer.
#[test]
fn auto_web_needs_an_answer() {
    let fixture = Fixture::new();
    let writer = SettingsWriter::with_path(&fixture.db, Arc::new(TurnGuard::new()));
    let error = eval_profile::apply(&writer, &fixture.resolved(), &repo_profile(), None)
        .expect_err("auto without a probe answer");
    assert!(error.to_string().contains("--web"), "{error}");
}

fn cli(home: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_litecode"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home.join("local"))
        .env_remove("LITECODE_TOKEN")
        .output()
        .expect("run the litecode binary")
}

/// The artifact side: the CLI writes the applied profile as one document, with
/// the web answer resolved and the tier it actually wrote.
#[test]
fn the_cli_writes_the_applied_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    let written = dir.path().join("applied.json");
    let output = cli(
        dir.path(),
        &[
            "--workspace",
            dir.path().to_str().expect("utf8 path"),
            "config",
            "profile",
            "apply",
            repo_profile_path().to_str().expect("utf8 path"),
            "--web",
            "on",
            "--write-applied",
            written.to_str().expect("utf8 path"),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stdout: {stdout}");
    assert!(stdout.contains("applied: "), "{stdout}");

    let text = std::fs::read_to_string(&written).expect("the document exists");
    let document: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(document["name"], "harbor");
    assert_eq!(document["web"], "on");
    assert_eq!(document["permissions"]["exec"], "all");
    let mut expected = repo_profile().tools.on;
    expected.push("webfetch".to_string());
    expected.push("websearch".to_string());
    expected.sort();
    let tools: Vec<String> = document["tools"]
        .as_array()
        .expect("tools is a list")
        .iter()
        .map(|value| value.as_str().expect("tool id").to_string())
        .collect();
    assert_eq!(tools, expected);
}

/// The key travels as a file, never as a value: no stdout, no applied document,
/// and the masked view of the settings shows that it is set.
#[test]
fn the_cli_stores_the_websearch_key_without_echoing_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let key = "exa-secret-0123456789";
    let key_file = dir.path().join("exa.key");
    std::fs::write(&key_file, format!("{key}\n")).expect("write key file");
    let written = dir.path().join("applied.json");
    let output = cli(
        dir.path(),
        &[
            "--workspace",
            dir.path().to_str().expect("utf8 path"),
            "config",
            "profile",
            "apply",
            repo_profile_path().to_str().expect("utf8 path"),
            "--web",
            "off",
            "--write-applied",
            written.to_str().expect("utf8 path"),
            "--websearch-key-file",
            key_file.to_str().expect("utf8 path"),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains(key),
        "the key must not be printed: {stdout}"
    );
    let document = std::fs::read_to_string(&written).expect("the document exists");
    assert!(
        !document.contains(key),
        "the key must not reach the applied document"
    );

    let database = cli_database(&stdout);
    let settings = ConfigManager::load_global_from(&database).expect("load settings");
    assert_eq!(settings.websearch.api_key.as_deref(), Some(key));
    let writer = SettingsWriter::with_path(&database, Arc::new(TurnGuard::new()));
    let masked = writer
        .websearch_view()
        .expect("masked view")
        .api_key
        .expect("a key is set");
    assert!(masked.contains("***"), "masked, not raw: {masked}");
    assert_ne!(masked, key);
}

/// A key file that is missing or empty is refused before anything is written.
#[test]
fn a_bad_websearch_key_file_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = dir.path().join("empty.key");
    std::fs::write(&empty, "\n").expect("write empty key file");
    let missing = dir.path().join("absent.key");
    for (key_file, needle) in [(empty, "is empty"), (missing, "absent.key")] {
        let output = cli(
            dir.path(),
            &[
                "--workspace",
                dir.path().to_str().expect("utf8 path"),
                "config",
                "profile",
                "apply",
                repo_profile_path().to_str().expect("utf8 path"),
                "--web",
                "off",
                "--websearch-key-file",
                key_file.to_str().expect("utf8 path"),
            ],
        );
        assert!(!output.status.success(), "{key_file:?} must be refused");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{stderr}");
    }
}

/// The line the CLI prints about its landing spot, read back as a path.
fn cli_database(stdout: &str) -> PathBuf {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("database: "))
        .map(PathBuf::from)
        .expect("the CLI names the database it wrote")
}

/// Acceptance 2, end to end: the command exits non-zero and the message names the
/// offending id.
#[test]
fn the_cli_exits_non_zero_and_names_the_unknown_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("bad.toml");
    std::fs::write(
        &profile,
        r#"name = "bad"
[tools]
on = ["read", "reed"]
[permissions]
read = "all"
"#,
    )
    .expect("write profile");
    let output = cli(
        dir.path(),
        &[
            "--workspace",
            dir.path().to_str().expect("utf8 path"),
            "config",
            "profile",
            "apply",
            profile.to_str().expect("utf8 path"),
            "--web",
            "off",
        ],
    );
    assert!(
        !output.status.success(),
        "a profile with a bad id must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("'reed'"), "{stderr}");
}

/// The command also works on the real file, in an isolated home.
#[test]
fn the_cli_applies_the_repository_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = cli(
        dir.path(),
        &[
            "--workspace",
            dir.path().to_str().expect("utf8 path"),
            "config",
            "profile",
            "apply",
            repo_profile_path().to_str().expect("utf8 path"),
            "--web",
            "off",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("profile 'harbor' applied"), "{stdout}");
    assert!(stdout.contains("web off"), "{stdout}");
    // The line names the database that took the bindings, so a stray run on a
    // developer machine is visible instead of silent.
    let database_line = stdout
        .lines()
        .find(|line| line.starts_with("database: "))
        .unwrap_or_else(|| panic!("no database line in:\n{stdout}"));
    assert!(
        database_line.contains(&dir.path().display().to_string()),
        "the database line points into the isolated HOME: {database_line}"
    );
    assert!(database_line.ends_with("litecode.db"), "{database_line}");
}

/// A profile that closes core cards and downgrades the shell. None of that is
/// reachable through a settings write any more (I1), which is exactly why the
/// evaluator needs its own channel (I6).
const ABLATED: &str = r#"name = "ablated"
[tools]
on = ["read", "grep", "glob", "exec_command", "run_code"]
off = [
  "write",
  "apply_edits",
  "plan.create",
  "plan.finish",
  "memory.list",
  "memory.upsert",
  "memory.delete",
  "subagent.launch",
  "subagent.wait",
  "subagent.stop",
  "subagent.list",
  "subagent.send",
  "todo.write",
  "ask_user",
]
[permissions]
read = "all"
grep = "all"
glob = "all"
exec = "safe"
"#;

/// I6, the sharp end: an ablation closes core cards, and a normal settings write
/// cannot undo it or erase it.
#[test]
fn an_ablated_profile_closes_core_cards_and_a_settings_write_leaves_it_alone() {
    let fixture = Fixture::new();
    let profile = EvalProfile::parse(ABLATED).expect("the ablated profile parses");
    fixture.apply(&profile, WebMode::Off).expect("apply");

    let surface = fixture.eval_surface().expect("the override is stored");
    assert_eq!(surface.agent_id, AGENT_ID);
    assert_eq!(surface.cards.get("exec"), Some(&Some(ToolPreset::Safe)));

    // The tool map is exactly the profile: closing `plan` / `memory` /
    // `subagent_tools` / `write` / `edit` is the part a settings write could not
    // do.
    let tools = fixture.default_tools();
    let mut cards: Vec<&str> = tools.keys().map(String::as_str).collect();
    cards.sort_unstable();
    assert_eq!(cards, ["exec", "glob", "grep", "read"]);
    assert_eq!(tools["exec"].preset, Some(ToolPreset::Safe));
    assert_eq!(tools["read"].preset, Some(ToolPreset::All));

    // The runtime agrees: the closed cards are gone from the model's view.
    let resolved = fixture.resolved();
    let catalog = capability_catalog(&resolved);
    let visible: BTreeSet<String> = catalog
        .selected_for_agent(&resolved, AGENT_ID, 0)
        .iter()
        .flat_map(|capability| capability.member_ids())
        .map(str::to_string)
        .collect();
    assert_eq!(
        resolved.agents()[AGENT_ID].tools["exec"].preset,
        Some(ToolPreset::Safe),
        "the tier survived resolution"
    );
    assert!(visible.contains("exec_command"), "{visible:?}");
    for member in [
        "plan.create",
        "memory.list",
        "subagent.launch",
        "apply_edits",
        "write",
        "todo.write",
    ] {
        assert!(!visible.contains(member), "'{member}' is closed: {visible:?}");
    }

    // A settings write touches the user's non-core intent only: the evaluator's
    // override is neither rewritten nor erased.
    let writer = SettingsWriter::with_path(&fixture.db, Arc::new(TurnGuard::new()));
    let mut request = writer
        .load_settings()
        .expect("load")
        .agents
        .get(AGENT_ID)
        .cloned()
        .expect("default agent");
    request.tools.insert(
        "code_search".to_string(),
        AgentToolBinding {
            enabled: false,
            preset: None,
            allowed_tools: None,
        },
    );
    writer
        .write_agent(AGENT_ID, request)
        .expect("a settings write succeeds");

    assert_eq!(
        fixture.eval_surface(),
        Some(surface),
        "write_agent must not touch meta.eval_surface"
    );
    assert_eq!(
        fixture.agent_tool_row("code_search"),
        Some(false),
        "the user's switch was recorded"
    );
    assert_eq!(
        fixture.default_tools(),
        tools,
        "the scored surface is unchanged by a settings write"
    );
}

/// Clearing the override restores the locked built-in surface and hands the
/// user's own non-core switches back untouched.
#[test]
fn clearing_restores_the_locked_surface_and_keeps_user_toggles() {
    let fixture = Fixture::new();
    let writer = SettingsWriter::with_path(&fixture.db, Arc::new(TurnGuard::new()));

    // The user's own intent, recorded through the settings path before any
    // profile runs.
    let mut request = writer
        .load_settings()
        .expect("load")
        .agents
        .get(AGENT_ID)
        .cloned()
        .expect("default agent");
    request.tools.insert(
        "code_search".to_string(),
        AgentToolBinding {
            enabled: false,
            preset: None,
            allowed_tools: None,
        },
    );
    writer.write_agent(AGENT_ID, request).expect("user toggle");

    let profile = EvalProfile::parse(ABLATED).expect("parses");
    fixture.apply(&profile, WebMode::On).expect("apply");
    assert!(
        fixture.eval_surface().is_some(),
        "the override is in force before the clear"
    );

    assert!(eval_profile::clear(&writer).expect("clear"), "one row went");
    assert_eq!(fixture.eval_surface(), None, "meta.eval_surface is gone");

    let restored = fixture.default_tools();
    for card in [
        "read",
        "write",
        "edit",
        "plan",
        "memory",
        "subagent_tools",
        "todo",
    ] {
        assert!(
            restored.get(card).is_some_and(|binding| binding.enabled),
            "'{card}' is back from the spec"
        );
    }
    assert_eq!(
        restored["exec"].preset,
        Some(ToolPreset::All),
        "the spec's tier is back"
    );
    assert!(
        !restored["code_search"].enabled,
        "the user's own off survived the whole round trip"
    );

    // Nothing left to clear, and no error about it.
    assert!(!eval_profile::clear(&writer).expect("a second clear is a no-op"));
}

/// The same round trip through the CLI: `config profile clear` removes the
/// override and says so, then reports there was nothing left.
#[test]
fn the_cli_clears_the_eval_surface() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().to_str().expect("utf8 path");
    let applied = cli(
        dir.path(),
        &[
            "--workspace",
            workspace,
            "config",
            "profile",
            "apply",
            repo_profile_path().to_str().expect("utf8 path"),
            "--web",
            "off",
        ],
    );
    let stdout = String::from_utf8_lossy(&applied.stdout);
    assert!(
        applied.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let database = cli_database(&stdout);
    {
        let conn = rusqlite::Connection::open(&database).expect("open db");
        assert!(
            store::load_eval_surface(&conn).expect("read").is_some(),
            "the apply stored an override"
        );
    }

    let cleared = cli(
        dir.path(),
        &["--workspace", workspace, "config", "profile", "clear"],
    );
    let stdout = String::from_utf8_lossy(&cleared.stdout);
    assert!(
        cleared.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert!(
        stdout.contains("eval surface override cleared"),
        "{stdout}"
    );
    assert!(stdout.contains("database: "), "{stdout}");
    {
        let conn = rusqlite::Connection::open(&database).expect("open db");
        assert!(
            store::load_eval_surface(&conn).expect("read").is_none(),
            "the override is gone from the same database"
        );
    }

    let again = cli(
        dir.path(),
        &["--workspace", workspace, "config", "profile", "clear"],
    );
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(
        again.status.success(),
        "clearing twice is not an error: {stdout}"
    );
    assert!(
        stdout.contains("no eval surface override to clear"),
        "{stdout}"
    );
}
