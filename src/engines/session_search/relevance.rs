//! The ranking judgment set: the fixed corpus and the queries whose *order* the
//! lane is expected to get right, with the metrics that say whether it did.
//!
//! Stage A of the ranking remediation. Every later change to a gate, a band or a
//! fusion weight has to keep this board green, which is the only reason the
//! numbers in `ranking::LAYERS` and `query_plan` are allowed to be numbers: they
//! are pinned against a corpus and a set of judgments, not against a feeling.
//!
//! The corpus is synthetic and fixed (session ids `S1`…`S7`, every row addressed
//! by `seq`), built through the real `sparse::build_index`, so it exercises the
//! real derivation — admission, roles, chunking — rather than a hand-written
//! index. The judgments are `query → (relevant rows, trap rows)`:
//!
//! * a **relevant** row is one a reader would have wanted;
//! * a **trap** is a row the old OR-everything ladder returned for that query and
//!   that a reader would not have wanted. Traps are what the gates exist for, so
//!   they are asserted to be *absent*, not merely ranked lower.
//!
//! Metrics are the standard ones: Recall@10, Precision@5, MRR and nDCG@10. They
//! are reported per query and asserted as floors, so a regression names the query
//! it broke instead of failing one aggregate number.



use super::sparse::{self, Lane};
















/// A live-corpus probe. **Ignored by default** because it needs a real
/// workspace and builds a real index (tens of seconds), and because it asserts
/// nothing: it prints what the lane does with the queries that started this
/// work, so the change can be reviewed against the corpus that motivated it.
///
/// ```text
/// cargo test --lib live_corpus_probe -- --ignored --nocapture
/// ```
///
/// It reads the workspace's `sessions.db` and builds into a temp dir: the live
/// index is never touched, so running it cannot disturb a running agent.
#[test]
#[ignore = "needs a real workspace; run on demand"]
fn live_corpus_probe() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = root.join(".litecode").join("sessions.db");
    if !db.is_file() {
        eprintln!("no live store at {}; nothing to probe", db.display());
        return;
    }
    let reader = crate::session::SessionDataReader::open(&db);
    let rows = reader
        .searchable_rows_blocking(None)
        .expect("read the live corpus");
    let data_root = reader.data_root().to_path_buf();
    // A row whose blob is gone cannot be derived, and the real build refuses the
    // whole batch over it (deliberately: a silent hole is worse than a loud
    // failure). This probe is a diagnostic, so it separates those rows out and
    // says so — the same list is what a rebuild would stop on.
    let mut readable = Vec::with_capacity(rows.len());
    let mut unreadable: Vec<String> = Vec::new();
    for row in rows {
        match crate::session::transcript_file::row_plain_text_strict(&row, &data_root) {
            Ok(_) => readable.push(row),
            Err(err) => unreadable.push(format!("{}:{} ({err})", row.session_id, row.seq)),
        }
    }
    eprintln!(
        "live corpus: {} rows readable, {} unreadable",
        readable.len(),
        unreadable.len()
    );
    for row in unreadable.iter().take(5) {
        eprintln!("  unreadable: {row}");
    }

    let dir = tempfile::TempDir::new().unwrap();
    let scratch = dir.path().join("session-index").join("sparse.db");
    // Derived against the live data root (where the blobs are) but written into
    // the temp dir, so a running agent's index is never touched.
    sparse::build_index_at(&readable, &data_root, &scratch).expect("build a scratch index");
    let index = sparse::open_read_only(&scratch).unwrap();

    for query in [
        "context compression",
        "the auth refactor token",
        "sparse lane noise",
        "为什么稀疏检索有噪音",
        // The Chinese queries this work started from: the long natural sentence
        // that used to return nothing, the fragment that used to be a phantom
        // hit, the two-word pair that used to need adjacency, and the mixed
        // pair that used to be answered by its Latin half alone.
        "重试次数能不能放大一点",
        "网络连接总是断掉",
        "配置落盘",
        "落盘",
        "网络连接",
        "网络连接重试逻辑",
        "codex 感知",
        "retry 重试",
    ] {
        let hits = index.search(Lane::Final, query, 60).expect("search");
        eprintln!("\n=== {query:?} ({} hits)", hits.len());
        // The sessions behind the hits, in rank order: the board's own corpus
        // contains sessions that quote these probes verbatim, so which sessions
        // answer is more informative than the first eight rows.
        let mut sessions: Vec<&str> = Vec::new();
        for hit in &hits {
            if !sessions.contains(&hit.session_id.as_str()) {
                sessions.push(&hit.session_id);
            }
        }
        eprintln!("  sessions: {sessions:?}");
        // A session that quotes the probe back (an evaluation transcript, say)
        // can fill the whole board with exact hits. The rows outside the loudest
        // session are what says whether the lane found the corpus this work was
        // about.
        let loudest = sessions
            .iter()
            .max_by_key(|sid| hits.iter().filter(|h| h.session_id == **sid).count())
            .copied();
        for hit in hits
            .iter()
            .filter(|h| Some(h.session_id.as_str()) != loudest)
            .take(5)
        {
            let layers: Vec<&str> = hit.evidence.iter().map(|e| e.layer.as_str()).collect();
            eprintln!(
                "  other: {} band={:?} strength={} role={:?} layers={:?}",
                hit.row_key, hit.rank.band, hit.rank.strength, hit.role, layers
            );
        }
        for hit in hits.iter().take(4) {
            let layers: Vec<&str> = hit.evidence.iter().map(|e| e.layer.as_str()).collect();
            eprintln!(
                "  {} band={:?} strength={} role={:?} layers={:?}",
                hit.row_key, hit.rank.band, hit.rank.strength, hit.role, layers
            );
        }
        // Full coverage of the query's informative words, for the rows that
        // survived: the number that says whether the noise is gone.
        let full = hits
            .iter()
            .filter(|h| {
                let best = h.evidence.iter().map(|e| e.coverage()).fold(0.0, f64::max);
                best >= 1.0
            })
            .count();
        eprintln!("  full coverage: {full}/{}", hits.len());
    }
}

