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

use std::collections::BTreeSet;
use std::path::Path;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
};
use crate::session::transcript_file::SearchableRow;
use crate::types::{Item, assistant_text, user_text};

use super::sparse::{self, Lane};

/// How deep the lane is asked for. Deeper than any assertion below: the point is
/// to see the whole ranked list, not to guess the cutoff.
const DEPTH: usize = 50;

/// Rank cutoffs the metrics are computed at.
const K_RECALL: usize = 10;
const K_PRECISION: usize = 5;
const K_NDCG: usize = 10;

fn row(sid: &str, seq: i64, kind: &str, item: &Item) -> SearchableRow {
    let value = serde_json::to_value(item).expect("serialize item");
    let item_type = value
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown")
        .to_string();
    SearchableRow {
        session_id: sid.into(),
        seq,
        kind: kind.into(),
        item_type,
        body: Some(value.to_string()),
        body_ref: None,
    }
}

fn call(sid: &str, seq: i64, call_id: &str, name: &str, arguments: &str) -> SearchableRow {
    row(
        sid,
        seq,
        "item/tool_call",
        &Item::FunctionCall(FunctionToolCall {
            arguments: arguments.into(),
            call_id: call_id.into(),
            namespace: None,
            name: name.into(),
            id: None,
            status: None,
        }),
    )
}

fn result(sid: &str, seq: i64, call_id: &str, output: &str) -> SearchableRow {
    row(
        sid,
        seq,
        "item/tool_result",
        &Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Text(output.into()),
            id: None,
            status: None,
        }),
    )
}

/// The fixed corpus. Every row is deterministic and addressed by `seq`.
///
/// It is built to contain, for each judged query, exactly one row that answers
/// it plus at least one row that shares surface vocabulary without answering it.
fn corpus() -> Vec<SearchableRow> {
    vec![
        // S1 — an ordinary turn about the auth module, without the query's noun.
        row("S1", 0, "item/user", &user_text("please refactor the auth module")),
        row("S1", 1, "item/assistant", &assistant_text("the auth module now has three files")),
        result("S1", 2, "c1", "compiled 12 files, 0 errors"),
        // S2 — the row the four-word query is actually about, plus a row made of
        // nothing but the query's glue.
        row(
            "S2",
            0,
            "item/assistant",
            &assistant_text("we decided the auth refactor token here, once, finally"),
        ),
        row("S2", 1, "item/user", &user_text("the the the the the the")),
        // S3 — a tool call and its output sharing one literal: which of the two a
        // question about it should prefer is a product decision, and the corpus
        // holds both so the decision is testable.
        call("S3", 0, "c3", "bash", r#"{"command":"grep NEEDLE_X src"}"#),
        result("S3", 1, "c3", "NEEDLE_X: 12 matches in 4 files"),
        // S4 — CJK: a full sentence and a fragment of it.
        row("S4", 0, "item/user", &user_text("稀疏检索的噪音来自单词命中，不是排序")),
        row("S4", 1, "item/assistant", &assistant_text("稀疏检索")),
        // S5 — a literal with one typo in it, plus prose sharing its shape.
        row("S5", 0, "item/user", &user_text("unique_session_phrase lives here")),
        row("S5", 1, "item/assistant", &assistant_text("unrelated prose about compression")),
        // S6 — the two-word query: one row is about it, one only shares a word.
        row("S6", 0, "item/assistant", &assistant_text("the compression ratio is four to one")),
        row("S6", 1, "item/user", &user_text("context compression happens at the turn boundary")),
        // S7 — a long row that mentions the two-word query's word repeatedly:
        // length alone must not buy it a place above the row that answers it.
        row(
            "S7",
            0,
            "item/assistant",
            &assistant_text(&format!("{} compression of the log", "filler ".repeat(200))),
        ),
    ]
}

fn build(dir: &Path) {
    let rows = corpus();
    sparse::build_index(&rows, dir).expect("build the judgment corpus");
}

fn ranked(dir: &Path, query: &str, scope: Option<&str>) -> Vec<String> {
    let index = sparse::open_read_only(&sparse::sparse_index_path(dir))
        .expect("open index")
        .with_scope(scope);
    index
        .search(Lane::Final, query, DEPTH)
        .expect("search")
        .into_iter()
        .map(|h| h.row_key)
        .collect()
}

/// One judged query: what a reader wanted, and what they did not.
struct Judgment {
    query: &'static str,
    /// Rows that answer the query. The first is expected to rank first.
    relevant: &'static [&'static str],
    /// Rows that answer it less completely — a subset of the query's words, say.
    /// They count as hits for precision and recall, but never outrank a primary
    /// row.
    secondary: &'static [&'static str],
    /// Rows the query's words appear in but that do not answer it. A trap must be
    /// absent from the answer entirely, not merely ranked low.
    traps: &'static [&'static str],
    /// nDCG@10 floor. Every query is judged on the same corpus, so a floor per
    /// query is what keeps a single bad ranking visible.
    min_ndcg: f64,
}

/// The judgments. Each one names the trap it exists to catch.
const JUDGMENTS: &[Judgment] = &[
    Judgment {
        // The full phrase. Its noun is what makes it a question.
        query: "auth refactor token",
        relevant: &["S2:0"],
        // S1:0 has two of the three words: a partial answer, not noise — and
        // never ranked above the row that has all three.
        secondary: &["S1:0"],
        // S1:1 has one word; S2:1 is made of the query's glue.
        traps: &["S1:1", "S2:1"],
        min_ndcg: 1.0,
    },
    Judgment {
        // Two words are an AND: sharing `compression` is not answering.
        query: "context compression",
        relevant: &["S6:1"],
        secondary: &[],
        // S6:0 and S7:0 share one word of two; S5:1 shares the other, in a row
        // that is otherwise about something else entirely. S7 is three hundred
        // words of filler, which the old length-blind ordering rewarded.
        traps: &["S6:0", "S7:0", "S5:1"],
        min_ndcg: 1.0,
    },
    Judgment {
        // An identifier with one wrong character: typo tolerance is the only
        // mechanism that may answer, and it must answer with the right row.
        query: "unique_session_phraze",
        relevant: &["S5:0"],
        secondary: &[],
        traps: &["S5:1"],
        min_ndcg: 1.0,
    },
    Judgment {
        // A CJK question: the fragment row must not stand in for the sentence.
        query: "稀疏检索的噪音来自单词命中",
        relevant: &["S4:0"],
        secondary: &[],
        traps: &["S4:1"],
        min_ndcg: 1.0,
    },
    Judgment {
        // A literal shared by a call and its output: both answer, and the call
        // (an intent record) comes first.
        query: "NEEDLE_X",
        relevant: &["S3:0", "S3:1"],
        secondary: &[],
        traps: &[],
        min_ndcg: 0.9,
    },
    Judgment {
        // Two words, no glue: both must be there, and a row with only one of
        // them is a trap rather than a weaker answer.
        query: "auth refactor",
        relevant: &["S2:0"],
        secondary: &["S1:0"],
        traps: &["S1:1"],
        min_ndcg: 0.8,
    },
];

/// Judgments are relative to the row, not the session: a hit inside a relevant
/// session but on a different row is not a hit.
fn hit_set(j: &Judgment) -> BTreeSet<&'static str> {
    j.relevant.iter().chain(j.secondary).copied().collect()
}

fn recall_at(ranked: &[String], relevant: &BTreeSet<&'static str>, k: usize) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let found = ranked
        .iter()
        .take(k)
        .filter(|key| relevant.contains(key.as_str()))
        .collect::<BTreeSet<_>>()
        .len();
    found as f64 / relevant.len() as f64
}

fn precision_at(ranked: &[String], relevant: &BTreeSet<&'static str>, k: usize) -> f64 {
    let seen = ranked.iter().take(k).count();
    if seen == 0 {
        return 1.0;
    }
    let good = ranked
        .iter()
        .take(k)
        .filter(|key| relevant.contains(key.as_str()))
        .count();
    good as f64 / seen as f64
}

/// Reciprocal rank of the first relevant hit: `0.0` when none is in the list.
fn reciprocal_rank(ranked: &[String], relevant: &BTreeSet<&'static str>) -> f64 {
    ranked
        .iter()
        .position(|key| relevant.contains(key.as_str()))
        .map(|index| 1.0 / (index as f64 + 1.0))
        .unwrap_or(0.0)
}

/// nDCG@k with binary gains. The ideal ordering is every relevant row first.
fn ndcg_at(ranked: &[String], relevant: &BTreeSet<&'static str>, k: usize) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .filter(|(_, key)| relevant.contains(key.as_str()))
        .map(|(index, _)| 1.0 / ((index as f64 + 2.0).log2()))
        .sum();
    let ideal: f64 = (0..relevant.len().min(k))
        .map(|index| 1.0 / ((index as f64 + 2.0).log2()))
        .sum();
    if ideal == 0.0 { 1.0 } else { dcg / ideal }
}

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
    ] {
        let hits = index.search(Lane::Final, query, 20).expect("search");
        eprintln!("\n=== {query:?} ({} hits)", hits.len());
        for hit in hits.iter().take(8) {
            let layers: Vec<&str> = hit
                .evidence
                .iter()
                .map(|e| e.layer.as_str())
                .collect();
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

/// A readable board for a failure message: the ranked list plus, for every hit,
/// why it is there.
fn board(dir: &Path) -> String {
    let index = sparse::open_read_only(&sparse::sparse_index_path(dir)).expect("open index");
    let mut out = String::new();
    for j in JUDGMENTS {
        out.push_str(&format!("\nquery {:?}\n", j.query));
        let hits = index.search(Lane::Final, j.query, DEPTH).expect("search");
        for hit in &hits {
            let mut layers = String::new();
            for evidence in &hit.evidence {
                layers.push_str(&format!(
                    " {}[{}/{},c{:.2},r{}]",
                    evidence.layer.as_str(),
                    evidence.matched,
                    evidence.total,
                    evidence.coverage(),
                    evidence.local_rank
                ));
            }
            out.push_str(&format!(
                "  {} band={:?} strength={} role={:?} rank={} ->{}\n",
                hit.row_key,
                hit.rank.band,
                hit.rank.strength,
                hit.role,
                hit.rank.local_rank,
                layers
            ));
        }
    }
    out
}

#[test]
fn the_judgment_board_holds() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());
    let mut failures: Vec<String> = Vec::new();
    let mut total_ndcg = 0.0;

    for j in JUDGMENTS {
        let ranked = ranked(dir.path(), j.query, None);
        let relevant = hit_set(j);
        let recall = recall_at(&ranked, &relevant, K_RECALL);
        let precision = precision_at(&ranked, &relevant, K_PRECISION);
        let rr = reciprocal_rank(&ranked, &relevant);
        let ndcg = ndcg_at(&ranked, &relevant, K_NDCG);
        total_ndcg += ndcg;

        if recall < 1.0 {
            failures.push(format!(
                "{:?}: Recall@{K_RECALL} = {recall:.2} (ranked {ranked:?})",
                j.query
            ));
        }
        if precision < 1.0 {
            failures.push(format!(
                "{:?}: Precision@{K_PRECISION} = {precision:.2} (ranked {ranked:?})",
                j.query
            ));
        }
        if rr < 1.0 {
            failures.push(format!(
                "{:?}: the first hit is not relevant (MRR {rr:.2}, ranked {ranked:?})",
                j.query
            ));
        }
        if ndcg < j.min_ndcg {
            failures.push(format!(
                "{:?}: nDCG@{K_NDCG} = {ndcg:.2} < {:.2} (ranked {ranked:?})",
                j.query, j.min_ndcg
            ));
        }
        // A partial answer never outranks a complete one.
        let last_primary = j
            .relevant
            .iter()
            .filter_map(|key| ranked.iter().position(|r| r == key))
            .max();
        let first_secondary = j
            .secondary
            .iter()
            .filter_map(|key| ranked.iter().position(|r| r == key))
            .min();
        if let (Some(primary), Some(secondary)) = (last_primary, first_secondary)
            && primary > secondary
        {
            failures.push(format!(
                "{:?}: a partial answer outranks the full one (ranked {ranked:?})",
                j.query
            ));
        }
        // The traps are the whole point: they must not be in the answer at all.
        for trap in j.traps {
            if ranked.iter().any(|key| key == trap) {
                failures.push(format!(
                    "{:?}: trap {trap} was returned (ranked {ranked:?})",
                    j.query
                ));
            }
        }
    }

    let mean_ndcg = total_ndcg / JUDGMENTS.len() as f64;
    if mean_ndcg < 0.95 {
        failures.push(format!("mean nDCG@{K_NDCG} = {mean_ndcg:.3} < 0.95"));
    }
    assert!(
        failures.is_empty(),
        "the judgment board regressed:\n{}\n{}",
        failures.join("\n"),
        board(dir.path())
    );
}

#[test]
fn a_narrowing_scope_keeps_the_same_order_within_the_session() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());
    let unscoped = ranked(dir.path(), "auth refactor token", None)
        .into_iter()
        .filter(|key| key.starts_with("S2:"))
        .collect::<Vec<_>>();
    let scoped = ranked(dir.path(), "auth refactor token", Some("S2"));
    assert_eq!(unscoped, scoped, "a session scope narrows, it does not re-rank");
}

#[test]
fn the_evidence_behind_each_judged_hit_names_its_layer() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());
    let index = sparse::open_read_only(&sparse::sparse_index_path(dir.path())).unwrap();

    let hits = index.search(Lane::Final, "auth refactor token", DEPTH).unwrap();
    let best = hits.iter().find(|h| h.row_key == "S2:0").expect("the answer");
    assert_eq!(
        best.rank.band,
        super::ranking::RankBand::Exact,
        "the literal is present, so the answer is exact evidence"
    );
    assert_eq!(best.coverage_of(super::ranking::LayerId::Exact), Some(1.0));

    // The literal and the words agree: more than one layer found it.
    assert!(
        best.layer_count() >= 2,
        "provenance was thrown away: {:?}",
        best.evidence
    );

    // A tool call outranks its own output at equal evidence: intent over outcome.
    let hits = index.search(Lane::Final, "NEEDLE_X", DEPTH).unwrap();
    let keys: Vec<&str> = hits.iter().map(|h| h.row_key.as_str()).collect();
    assert_eq!(keys, ["S3:0", "S3:1"], "{hits:#?}");
    assert_eq!(hits[0].role, super::ranking::ContentRole::Action);
    assert_eq!(hits[1].role, super::ranking::ContentRole::Outcome);
}
