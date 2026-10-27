//! Structural regression gates for the compacted-context refactor.
//!
//! These tests intentionally inspect source text: the refactor's dangerous
//! failures are architectural (a second execute path, a hidden agent binding,
//! or a second projection/role source), not a value that a normal unit test
//! can observe reliably.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|error| panic!("read {rel}: {error}"))
}

/// Production portion of a Rust source file. Test-only fixtures may mention
/// legacy/removed names to prove compatibility; they are not shipped paths.
fn production_part(source: &str) -> &str {
    source
        .split("\n#[cfg(test)]\nmod tests {")
        .next()
        .unwrap_or(source)
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()))
    {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn source_files() -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    rust_sources(&root().join("src"), &mut paths);
    paths
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            (path, text)
        })
        .collect()
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn one_compaction_execute_path() {
    let compact = read("src/context_pipeline/compact.rs");
    let production = production_part(&compact);
    assert_eq!(
        production.matches("SessionMutation::Compact {").count(),
        1,
        "compaction must commit through exactly one SessionMutation::Compact site"
    );
    assert_eq!(
        production.matches("fn first_pass_compaction(").count(),
        1,
        "there must be one first-pass executor"
    );
    assert_eq!(
        production
            .matches("Self::call_llm_compact_once_with_timeout(")
            .count(),
        1,
        "the execute path must call the single provider-attempt helper exactly once"
    );
    for forbidden in [
        "fn second_pass",
        "fn call_llm_compact(",
        "fn call_llm_compact_with_timeout(",
        "AggressiveTruncate",
        "aggressive_truncate",
    ] {
        assert!(
            !production.contains(forbidden),
            "removed alternate compaction path remains: {forbidden}"
        );
    }
    assert_eq!(
        production
            .matches("fn call_llm_compact_once_with_timeout(")
            .count(),
        1,
        "the provider attempt must have one definition"
    );
}

#[test]
fn summary_user_projection_has_one_owner() {
    let mut owners = Vec::new();
    for (path, text) in source_files() {
        if text.contains("SUMMARY_OPEN_TAG") {
            owners.push(relative(&path));
        }
    }
    assert_eq!(
        owners,
        vec!["src/session/working.rs".to_string()],
        "only the working-set projector may assemble the checkpoint user message"
    );

    let working = read("src/session/working.rs");
    assert!(working.contains("pub fn model_item(&self) -> Item"));
    assert!(working.contains("user_text(format!("));
    assert!(working.contains("strip_legacy_summary_prefix"));

    for (path, text) in source_files() {
        let production = production_part(&text);
        let rel = relative(&path);
        for prefix in ["[Conversation summary]", "[Aggressive summary]"] {
            if production.contains(prefix) {
                assert_eq!(
                    rel, "src/session/working.rs",
                    "legacy summary prefix handling escaped the projector: {prefix}"
                );
            }
        }
    }
}

#[test]
fn one_context_meter_and_token_breakdown_source() {
    let sources = source_files();
    let owners = |needle: &str| {
        sources
            .iter()
            .filter(|(_, text)| text.contains(needle))
            .map(|(path, _)| relative(path))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        owners("pub struct ContextMeter"),
        vec!["src/context_pipeline/context_meter.rs".to_string()],
        "ContextMeter must have one definition"
    );
    assert_eq!(
        owners("pub struct ContextEnvelope"),
        vec!["src/context_pipeline/context_meter.rs".to_string()],
        "ContextEnvelope must have one definition"
    );
    assert_eq!(
        owners("pub struct ItemTokenBreakdown"),
        vec!["src/session/estimate.rs".to_string()],
        "token breakdown must have one authority type"
    );
    assert_eq!(
        owners("pub fn autocompact_threshold("),
        vec!["src/session/estimate.rs".to_string()],
        "auto threshold must have one formula"
    );
    assert_eq!(
        owners("pub fn compute_token_breakdown("),
        vec!["src/session/estimate.rs".to_string()],
        "breakdown math must have one implementation"
    );
    assert_eq!(
        owners("context_window as f64 * 0.8"),
        vec!["src/session/estimate.rs".to_string()],
        "the 0.8 threshold formula must live in the single estimate helper"
    );
}

#[test]
fn no_hidden_compaction_agent_or_prompt_binding() {
    assert!(
        !root()
            .join("src/config/global_db/prompts/compaction.md")
            .exists(),
        "the retired hidden summarizer prompt must not return"
    );
    for (path, text) in source_files() {
        let rel = relative(&path);
        assert!(
            !text.contains("COMPACTION_PROMPT"),
            "hidden compaction prompt symbol remains in {rel}"
        );
        assert!(
            !text.contains("builtin:compaction"),
            "hidden compaction system marker remains in {rel}"
        );
    }
    let seed = read("src/config/global_db/seed.rs");
    let seed_production = production_part(&seed);
    assert!(
        !seed_production.contains("\"compaction\",\n        AgentRole::Hidden"),
        "seed must not create a hidden compaction agent"
    );
    let bindings = read("src/tool/agent_bindings.rs");
    assert!(
        !bindings.contains("\"compaction\""),
        "agent binding normalization must not special-case a retired agent id"
    );
    for (path, text) in source_files() {
        assert!(
            !production_part(&text).contains("agent_id == \"compaction\""),
            "retired compaction agent-id branch remains in {}",
            relative(&path)
        );
    }
    let schema = read("src/config/schema.rs");
    assert!(schema.contains("pub const PROTECTED_AGENT_IDS: &[&str] = &[\"default\"];"));
    let settings = read("web/src/api/settings.ts");
    assert!(settings.contains("export const PROTECTED_AGENT_IDS = new Set([\"default\"]);"));
    assert!(!settings.contains("isHiddenSettingsAgent"));
    let agents_section = read("web/src/dockview/panels/settings/AgentsSection.tsx");
    assert!(!agents_section.contains("isHiddenSettingsAgent"));
    assert!(!agents_section.contains("builtin:compaction"));
}

#[test]
fn no_pi_previous_summary_branch_or_split_turn_state() {
    let mut paths = Vec::new();
    rust_sources(&root().join("src/context_pipeline"), &mut paths);
    rust_sources(&root().join("src/session"), &mut paths);
    paths.extend([
        root().join("src/llm/request.rs"),
        root().join("src/config/global_db/prompts/compaction_instruction.md"),
    ]);
    for path in paths {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for forbidden in [
            "<previous-summary>",
            "previous_summary",
            "split_turn",
            "split-turn",
            "branch_point",
            "branch_from",
        ] {
            assert!(
                !text.contains(forbidden),
                "Pi-style compaction state remains in {}: {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn legacy_no_caller_helpers_stay_deleted() {
    for (path, text) in source_files() {
        for symbol in [
            "spine_agent_item",
            "render_for_summary",
            "build_compaction_prompt",
            "ring_occupancy",
            "manual_compact_eligible",
            "working_set_items",
            "fn budget_items(",
            "resolve_compaction",
            "CompactLlmCall",
        ] {
            assert!(
                !text.contains(symbol),
                "retired symbol {symbol} remains in {}",
                relative(&path)
            );
        }
    }
    for (path, text) in source_files() {
        if let Some(start) = text.find("fn kind_for_item(") {
            let function = text[start..]
                .lines()
                .take_while(|line| *line != "}")
                .collect::<String>();
            assert!(
                !function.contains("SessionKind::Compacted"),
                "kind_for_item must not infer compacted payload identity in {}",
                relative(&path)
            );
        }
    }
}

#[test]
fn lifecycle_carries_unified_telemetry() {
    let observer = read("src/runtime/observer.rs");
    assert!(observer.contains("pub struct CompactionTelemetry"));
    assert!(observer.contains("telemetry: CompactionTelemetry"));
    for field in [
        "tokens_before",
        "tokens_pruned",
        "tokens_after",
        "summary_attempts",
        "overflow_retry",
    ] {
        assert!(
            observer.contains(&format!("pub {field}: Option<")),
            "missing telemetry field {field}"
        );
    }
    let compact = read("src/context_pipeline/compact.rs");
    assert!(compact.contains("emit_compact_lifecycle_with_telemetry("));
    assert!(compact.contains("CompactionTelemetry {"));
    assert!(
        compact.contains("overflow_retry: Some(u8::from(trigger == CompactionTrigger::Overflow))")
    );
}

#[test]
fn pr7_uses_provider_capacity_and_minimal_summary_acceptance() {
    let meter = read("src/context_pipeline/context_meter.rs");
    let production = production_part(&meter);
    assert!(production.contains("context_window.saturating_sub(output_reserve)"));
    assert!(production.contains("(context_window.saturating_mul(4) / 5).min(prompt_limit)"));
    for removed in ["safety_headroom", "MAX_SAFETY_HEADROOM_TOKENS"] {
        assert!(
            !production.contains(removed),
            "removed meter policy remains: {removed}"
        );
    }

    let compact = read("src/context_pipeline/compact.rs");
    let compact_production = production_part(&compact);
    for removed in [
        "SummaryAcceptanceConfig",
        "DSH_SUMMARY_SECTIONS",
        "summary_quality_threshold",
        "summary_min_saved_tokens",
        "latest_user_text",
        "recent_user_anchor",
    ] {
        assert!(
            !compact_production.contains(removed),
            "removed summary acceptance heuristic remains: {removed}"
        );
    }
    assert!(compact_production.contains("summary_tokens >= source_tokens"));
    assert!(compact_production.contains("pub const MAX_SUMMARY_ATTEMPTS: u8 = 2;"));
    assert_eq!(
        compact_production
            .matches("for attempt in 1..=max_attempts")
            .count(),
        1,
        "exactly one loop implements the attempt budget"
    );
    assert!(
        compact_production.contains("let max_attempts = max_attempts.max(1);"),
        "the attempt budget must never degrade into a zero-trip loop"
    );

    let observer = read("src/runtime/observer.rs");
    assert!(!observer.contains("review_score"));
}
