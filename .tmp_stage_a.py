from pathlib import Path
import re

p = Path("tests/stage_a_ctx_consistency.rs")
t = p.read_text(encoding="utf-8")

t = t.replace(
    """fn setup_workspace_and_session(dir: &std::path::Path, agent: &str) -> (String, String) {
    let ws = light_workspace(dir);
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let session = Session::open(&db_path, "/proj", agent, Some("test-model")).expect("open");
    let sid = session.id.clone();
    (db_path, sid)
}""",
    """fn setup_workspace_and_session(
    dir: &std::path::Path,
    agent: &str,
) -> (String, String, Arc<SessionManager>) {
    let ws = light_workspace(dir);
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let sessions = test_sessions(&db_path);
    let sid = sessions
        .open_session_sync("/proj", agent, Some("test-model"))
        .expect("open");
    (db_path, sid, sessions)
}""",
)

t = t.replace(
    "let (db_path, sid) = setup_workspace_and_session(",
    "let (db_path, sid, sessions) = setup_workspace_and_session(",
)

t = re.sub(
    r"\n    let sessions = test_sessions\(&db_path\);\n    sessions\.register_for_test\(Session::resume\(&db_path, &sid\)\.unwrap\(\)\);\n",
    "\n",
    t,
)
t = re.sub(
    r"\n    sessions\.register_for_test\(Session::resume\(&db_path, &sid\)\.unwrap\(\)\);\n",
    "\n",
    t,
)

t = re.sub(
    r"""    \{\n        let s = Session::resume\(&db_path, &sid\)\.(?:expect\("resume"\)|unwrap\(\));\n        s\.insert_detail_rows\((.*?)\n        \)\.unwrap\(\);\n    \}\n""",
    r"    sessions.insert_detail_rows(&sid, \1).unwrap();\n",
    t,
    flags=re.S,
)
t = re.sub(
    r"""    \{\n        let s = Session::resume\(&db_path, &sid\)\.(?:expect\("resume"\)|unwrap\(\));\n        s\.insert_detail_rows\(([^;]+?)\)\.unwrap\(\);\n    \}\n""",
    r"    sessions.insert_detail_rows(&sid, \1).unwrap();\n",
    t,
)

t = re.sub(
    r"    let session = Session::resume\(&db_path, &sid\)\.unwrap\(\);\n    session\.insert_detail_rows\(([^;]+?)\)\.unwrap\(\);\n",
    r"    sessions.insert_detail_rows(&sid, \1).unwrap();\n",
    t,
)

t = re.sub(
    r"\n    let session = Session::resume\(&db_path, &sid\)\.unwrap\(\);\n",
    "\n",
    t,
)
t = re.sub(
    r"\n    let resumed = Session::resume\(&db_path, &sid\)\.unwrap\(\);\n",
    "\n",
    t,
)
t = re.sub(
    r"\n    let session2 = Session::resume\(&db_path, &sid\)\.unwrap\(\);\n",
    "\n",
    t,
)

t = re.sub(r"ContextPipeline::new\(&session,\s*", "ContextPipeline::new(", t)
t = t.replace("begin_turn(&session)", "begin_turn(&sessions, &sid)")
t = t.replace("commit_step(&session,", "commit_step(&sessions, &sid,")
t = t.replace(
    "commit_step_from_items(&self.session,",
    "commit_step_from_items(&self.sessions, &self.session_id,",
)
t = t.replace("commit_step_from_items(&session,", "commit_step_from_items(&sessions, &sid,")

t = t.replace(
    """    sessions
        .with_entry_store(sid, |s| Ok(pipeline.commit_step(s, turn)?))
        .expect("commit via session gate")
""",
    """    pipeline
        .commit_step(sessions, sid, turn)
        .expect("commit via session")
""",
)

t = re.sub(r"\bsession\.load_transcript\(\)", "sessions.data().transcript_blocking(&sid).unwrap()", t)
t = re.sub(
    r"\bsession\.load_history_transcript\(\)",
    "sessions.data().transcript_blocking(&sid).unwrap()",
    t,
)
t = re.sub(r"\bsession\.load_events\(\)", "sessions.data().events_blocking(&sid).unwrap()", t)
t = re.sub(r"\bsession\.persisted_max_seq\(\)", "sessions.entry_wire_seq_cursor(&sid).0", t)
t = re.sub(
    r"\bsession\.revert_to_user_anchor\(",
    "sessions.entry_revert_to_user_anchor(&sid, ",
    t,
)
t = re.sub(r"\bsession\.insert_detail_rows\(", "sessions.insert_detail_rows(&sid, ", t)

t = t.replace(
    "deps.session.load_transcript()",
    "deps.sessions.data().transcript_blocking(&deps.session_id).unwrap()",
)
t = t.replace(
    "self.session.revert_to_user_anchor(k)",
    "self.sessions.entry_revert_to_user_anchor(&self.session_id, k)",
)

t = t.replace(
    """struct PipelinePersistDeps {
    pipeline: ContextPipeline,
    session: Session,
""",
    """struct PipelinePersistDeps {
    pipeline: ContextPipeline,
    sessions: Arc<SessionManager>,
    session_id: String,
""",
)
t = t.replace(
    """    let mut deps = PipelinePersistDeps {
        pipeline,
        session,
""",
    """    let mut deps = PipelinePersistDeps {
        pipeline,
        sessions: Arc::clone(&sessions),
        session_id: sid.clone(),
""",
)

t = t.replace(
    """    let session =
        Session::open(&db_path.to_string_lossy(), &project, "default", model_ref).unwrap();
    let session_id = session.id.clone();
    let sessions = Arc::new(SessionManager::new(
        Arc::new(TurnGuard::new()),
        db_path.to_string_lossy().to_string(),
    ));
    sessions.register_for_test(session);
""",
    """    let sessions = Arc::new(SessionManager::new(
        Arc::new(TurnGuard::new()),
        db_path.to_string_lossy().to_string(),
    ));
    let session_id = sessions
        .open_session_sync(&project, "default", model_ref)
        .unwrap();
""",
)

t = t.replace("use litecode::session::store::Session;\n", "")

# leftover Session::resume in register or comments
t = re.sub(r"Session::resume\(&db_path, &sid\)", "sessions.data().meta_blocking(&sid)", t)

p.write_text(t, encoding="utf-8")
print("stage_a rewritten, remaining Session::", t.count("Session::"))
print("remaining with_entry", t.count("with_entry_store"))
print("remaining .session", len(re.findall(r"\bsession\b", t)))
