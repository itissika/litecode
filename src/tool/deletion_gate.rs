//! Phase 0 `rg` gate for the direct-era deletion list.
//!
//! Every item runs. A hit is `red` (still present). No hit is `green`.
//! Phase 0 records red; it does not hide it with `#[ignore]`. Phase 5 is when
//! the ledger is required to be all green. Status is the assertion. Hit counts
//! are recorded and may move as surrounding lines change.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

struct Gate {
    id: &'static str,
    item: &'static str,
    /// Patterns that are OR-ed. A hit on any pattern counts.
    patterns: &'static [&'static str],
    paths: &'static [&'static str],
    /// When set, the item is red only if `patterns` hit and this pattern does not.
    absent_pattern: Option<&'static str>,
    absent_paths: &'static [&'static str],
}

const GATES: &[Gate] = &[
    Gate {
        id: "content-ok",
        item: "ToolCallResult content as the fact source in the five tools",
        patterns: &[r"ToolCallResult::ok\("],
        paths: &[
            "src/tools/read.rs",
            "src/tools/glob.rs",
            "src/tools/grep.rs",
            "src/tools/write.rs",
            "src/tools/edit.rs",
            "src/tools/edit/feedback.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "hint-warning-appendix",
        item: "hint / warning_status / appendix on the five tools",
        patterns: &[
            r"warning_status",
            r"\.appendix\b",
            r"with_warning\(",
            r"with_hint_appendix",
            r"apply_read_warnings",
            r"\.hint\s*=",
        ],
        paths: &[
            "src/tools/read.rs",
            "src/tools/glob.rs",
            "src/tools/grep.rs",
            "src/tools/write.rs",
            "src/tools/edit.rs",
            "src/tools/edit/feedback.rs",
            "src/tools/edit/matcher.rs",
            "src/tools/edit/planner.rs",
            "src/tools/file_path.rs",
            "src/tools/lsp_feedback.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "warning-level",
        item: "warning as an independent result level on the five tools",
        patterns: &[r"ToolSignalLevel::Warning"],
        paths: &[
            "src/tools/read.rs",
            "src/tools/glob.rs",
            "src/tools/grep.rs",
            "src/tools/write.rs",
            "src/tools/edit",
            "src/tools/file_path.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "file-path-alias",
        item: "historical file_path alias",
        patterns: &[r"accepts_historical_file_path|alias_code_path"],
        paths: &[
            "src/tools/file_path.rs",
            "src/tools/edit.rs",
            "src/tools/write.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "grep-view",
        item: "grep.view display parameter",
        patterns: &[r#""view""#],
        paths: &["src/tools/grep.rs", "src/tools/descriptions/grep.md"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "grep-context-lines",
        item: "grep.context_lines",
        patterns: &[r"context_lines"],
        paths: &["src/tools/grep.rs", "src/tools/descriptions/grep.md"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "read-line-text",
        item: "read line-number text and pagination footer",
        patterns: &[r"format_read_line|pagination_footer"],
        paths: &["src/tools/read.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "glob-list-text",
        item: "glob list text",
        patterns: &[r"format_glob_body"],
        paths: &["src/tools/glob.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "grep-text-view",
        item: "grep text view",
        patterns: &[r"enum GrepShape"],
        paths: &["src/tools/grep.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "edit-text-status",
        item: "edit text status rendering",
        patterns: &[r"format_batch_body"],
        paths: &["src/tools/edit"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "write-text-status",
        item: "write text status rendering",
        patterns: &[r"Created|Updated"],
        paths: &["src/tools/write.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "path-gate",
        item: "PathGate / path_gate_key",
        patterns: &[r"PathGate|path_gate"],
        paths: &["src/tools/code.rs", "src/tool/resource_gate.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "max-result-size",
        item: "max_result_size cap on the five-tool path",
        patterns: &[r"max_result_size"],
        paths: &[
            "src/tools/read.rs",
            "src/tools/grep.rs",
            "src/tool/executor.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "spill-emergency",
        item: "spill / emergency cap shared by the five tools",
        patterns: &[r"fn finalize_tool_call_result|apply_emergency_cap"],
        paths: &["src/tool/output.rs", "src/tool/executor.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "concurrency-safe",
        item: "is_concurrency_safe special case on the five tools",
        patterns: &[r"fn is_concurrency_safe"],
        paths: &[
            "src/tools/read.rs",
            "src/tools/glob.rs",
            "src/tools/grep.rs",
            "src/tools/edit.rs",
            "src/tools/write.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "fuzzy",
        item: "fuzzy matcher chain",
        patterns: &[
            r"(?i)\bfuzzy\b",
            r"MatchKind::Fuzzy",
            r"allow_fuzzy",
            r"resolve_fuzzy",
            r"edit_fuzzy",
            r"FuzzyBand",
            r"classify_fuzzy",
        ],
        paths: &["src/tools/edit", "src/tools/descriptions/edit.md"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "read-media",
        item: "read image detection / media parts / MAX_MEDIA_BLOB_SIZE",
        patterns: &[r"MAX_MEDIA_BLOB_SIZE|detect_image_mime|ReadData::Image"],
        paths: &["src/tools/read.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "token-cap",
        item: "read INTERNAL_TOKEN_CAP",
        patterns: &[r"INTERNAL_TOKEN_CAP"],
        paths: &["src/tools/read.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "grep-repair",
        item: "grep BraceText / Literal automatic repair",
        patterns: &[r"PatternMode::BraceText|PatternMode::Literal"],
        paths: &["src/tools/grep.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "per-call-options",
        item: "per-call options / split_invocation",
        patterns: &[r"split_invocation|parse_call_timeout_ms|ToolCallOptions"],
        paths: &["src/tools/code.rs", "src/tool/code_manifest.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "glob-prefix-strip",
        item: "glob path/pattern prefix stripping",
        patterns: &[r"fn strip_redundant_path_prefix"],
        paths: &["src/tools/glob.rs"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "edit-typography",
        item: "edit typography equivalence",
        patterns: &[
            r"UnicodeConfusable",
            r"confusable",
            r"CONFUSABLE",
            r"typography",
            r"smart quote",
            r"smart_quote",
        ],
        paths: &[
            "src/tools/edit",
            "src/tools/descriptions/edit.md",
            "src/workspace/text_codec.rs",
        ],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "edit-on-error-match",
        item: "edit on_error / match parameter",
        patterns: &[r"on_error", r#""match""#],
        paths: &["src/tools/edit", "src/tools/descriptions/edit.md"],
        absent_pattern: None,
        absent_paths: &[],
    },
    Gate {
        id: "schema-minimum-unenforced",
        item: "schema minimum declared on the five tools but not executed (D-10 still open; this only records the gap)",
        patterns: &[r#""minimum""#],
        paths: &[
            "src/tools/read.rs",
            "src/tools/glob.rs",
            "src/tools/grep.rs",
            "src/tools/edit.rs",
            "src/tools/write.rs",
        ],
        absent_pattern: Some(r#"get\("minimum"\)"#),
        absent_paths: &["src/tool/schema_validate.rs"],
    },
];

struct Search {
    lines: Vec<String>,
}

impl Search {
    fn empty() -> Self {
        Self { lines: Vec::new() }
    }

    fn hits(&self) -> usize {
        self.lines.len()
    }

    fn sample(&self) -> String {
        self.lines.first().cloned().unwrap_or_default()
    }
}

/// A test that asserts the symbol is absent (`assert!(!….contains("…"))`) is
/// not a leftover implementation.
fn is_negative_assertion(line: &str) -> bool {
    let text = match line.splitn(3, ':').nth(2) {
        Some(text) => text,
        None => line,
    };
    let trimmed = text.trim_start();
    trimmed.starts_with("assert!(!") && trimmed.contains(".contains(\"")
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn search(pattern: &str, rels: &[&str]) -> Result<Search, String> {
    if rels.is_empty() {
        return Ok(Search::empty());
    }
    let mut cmd = Command::new("rg");
    cmd.arg("-n")
        .arg("-H")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .arg(pattern);
    for rel in rels {
        let path = repo().join(rel);
        if !path.exists() {
            return Err(format!("deletion gate path missing: {}", path.display()));
        }
        cmd.arg(path);
    }
    let output = cmd.output().map_err(|error| {
        format!("rg failed to start ({error}). The deletion gate is not skipped.")
    })?;
    match output.status.code() {
        Some(0) | Some(1) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let root = repo();
            let root_display = root.to_string_lossy().replace('\\', "/");
            let lines = stdout
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| {
                    let mut owned = line.replace('\\', "/");
                    if let Some(stripped) = owned.strip_prefix(&root_display) {
                        owned = stripped.trim_start_matches('/').to_string();
                    }
                    if owned.chars().count() > 240 {
                        owned = owned.chars().take(240).collect();
                    }
                    owned
                })
                .collect();
            Ok(Search { lines })
        }
        Some(code) => Err(format!(
            "rg exited {code} for /{pattern}/: {}",
            String::from_utf8_lossy(&output.stderr)
        )),
        None => Err(format!("rg terminated by signal for /{pattern}/")),
    }
}

fn gate_status(gate: &Gate) -> Result<Value, String> {
    let mut hits = 0usize;
    let mut sample = String::new();
    for pattern in gate.patterns {
        let found = search(pattern, gate.paths)?;
        let lines: Vec<String> = if gate.id == "per-call-options" {
            found
                .lines
                .into_iter()
                .filter(|line| !is_negative_assertion(line))
                .collect()
        } else {
            found.lines
        };
        hits += lines.len();
        if sample.is_empty() {
            sample = lines.first().cloned().unwrap_or_default();
        }
    }
    let mut status = if hits == 0 { "green" } else { "red" };
    if let Some(absent) = gate.absent_pattern {
        let missing = search(absent, gate.absent_paths)?;
        if hits > 0 && missing.hits() == 0 {
            status = "red";
        } else {
            status = "green";
        }
        if sample.is_empty() {
            sample = missing.sample();
        }
    }
    Ok(json!({
        "id": gate.id,
        "item": gate.item,
        "status": status,
        "hits": hits,
        "sample": sample,
    }))
}

fn ledger_path() -> PathBuf {
    repo().join("src/tool/fixtures/deletion_gate.json")
}

#[test]
fn phase0_direct_legacy_deletion_gate() {
    let mut actual = Vec::with_capacity(GATES.len());
    for gate in GATES {
        let row = gate_status(gate)
            .unwrap_or_else(|error| panic!("deletion gate '{}' did not run: {error}", gate.id));
        eprintln!(
            "deletion-gate {} {} hits={}",
            row["status"].as_str().unwrap_or("?"),
            row["id"].as_str().unwrap_or("?"),
            row["hits"]
        );
        actual.push(row);
    }
    let path = ledger_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing deletion ledger {}: {error}", path.display()));
    let ledger: Value = serde_json::from_str(&text).expect("deletion ledger json");
    let rows = ledger["gates"]
        .as_array()
        .expect("deletion ledger gates array");
    assert_eq!(rows.len(), GATES.len(), "ledger row count");
    let mut mismatches = Vec::new();
    for (gate, expected) in GATES.iter().zip(rows) {
        let actual_row = actual.iter().find(|row| row["id"] == gate.id).expect("row");
        assert_eq!(expected["id"], gate.id, "ledger order");
        let expected_status = expected["status"].as_str().unwrap_or("");
        let actual_status = actual_row["status"].as_str().unwrap_or("");
        if expected_status != actual_status {
            mismatches.push(format!(
                "{} ledger={expected_status} actual={actual_status} hits={} sample={}",
                gate.id, actual_row["hits"], actual_row["sample"]
            ));
        }
        assert!(
            expected_status == "red" || expected_status == "green",
            "{} status {expected_status}",
            gate.id
        );
    }
    assert!(
        mismatches.is_empty(),
        "deletion gate drift (red is allowed in Phase 0 only when the ledger says red):\n{}",
        mismatches.join("\n")
    );
}
