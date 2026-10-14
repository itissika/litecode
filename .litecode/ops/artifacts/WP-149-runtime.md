# WP-149 delivery (runtime): primary agents bind code_search by default

Implements contract WP-145-backend.md §B (WP-146 v1). The plan is in .tmp/wp149_plan.md. Q1/Q3/Q4/Q5 use the defaults. Q2 was revised by 后端中层: write_agent auto-binds code_search for new primary agents.

## Behavior
- **Primary role:** `agents.role = 'primary'` (`AgentRole::Primary`), excluding `compaction`.
  - Qualifies: default, orchestrator, and custom primary agents.
  - Never bound: explore, general (Subagent), compaction (Hidden), and custom subagent or hidden agents.
- **New DB (`seed`):** `seed_primary_agent_bindings` inserts code_search (enabled, `binding_seeded(All)`) only when the row is missing. `seed` also writes the meta marker `default_bind_code_search_v1`, so a new DB never runs the backfill.
- **Existing DB:** `ensure_core_bindings` runs `ensure_code_search_default_v1` last, after `reconcile_role_rules` and `normalize_fixed_builtin_bindings`.
  - If the marker is present, it returns immediately.
  - Otherwise, in one transaction: `INSERT … SELECT FROM agents WHERE role='primary' AND NOT EXISTS(row) ON CONFLICT DO NOTHING`, then write the marker (`ON CONFLICT DO NOTHING`), then commit.
  - There is no UPDATE path, so existing true or false rows are untouched.
  - On error it rolls back with no marker and retries on the next startup.
  - `user_version` is unchanged (still 7), and `SEED_REVISION` is unchanged ("15").
- **New agents (Q2):** `SettingsWriter::write_agent` adds code_search=true when the id is new, the role is Primary, and the submitted tools map lacks the key. An explicit key (true or false) is written as given. An existing agent switched to primary is not auto-bound.

## Changed files
- src/config/global_db/tools.rs: `default_bound_optional_builtins() -> ["code_search"]`
- src/config/global_db/seed.rs:
  - seed marker
  - primary seed insert-if-missing
  - `default_optional_binding`, `CODE_SEARCH_DEFAULT_MARKER`, `mark_code_search_default_done`, `ensure_code_search_default_v1`
  - hook in `ensure_core_bindings`
  - 5 tests
- src/config/global_db/mod.rs: `pub(crate) use seed::default_optional_binding`; `store::path_mode_to_str` and `store::preset_to_str` changed to `pub(crate)`
- src/config/settings_writer.rs: new-primary auto-bind in `write_agent`, plus 3 tests

## Tests (cargo test -j2, scoped, shared target, MemAvailable ≥ 6 GiB checked before each run, about 6.4–6.6 GiB)
- `--lib config::global_db`: 29/29 ok
- `--lib config::settings_writer`: 26/26 ok
- New tests, all ok:
  - seed_binds_code_search_true_on_primary_agents
  - seed_does_not_bind_code_search_on_subagents
  - migration_fills_missing_primary_code_search_only
  - migration_keeps_existing_false_row
  - migration_runs_once_on_repeated_startup
  - write_agent_new_primary_missing_code_search_binds_true
  - write_agent_new_primary_explicit_false_stays_false
  - write_agent_existing_agent_switched_to_primary_not_auto_bound
- `--test f2_lock_scope`: 4/4 ok
- `--test settings_api`: 47/48. `settings_retrieval_engine_init_and_stop_use_engine_api` fails at tests/settings_api.rs:2095. It asserts that code_search disappears from available tools after the engine stops. WP-148 (Tools, src/tool/availability.rs) made code_search visibility depend only on the binding. With default=bound, it now stays visible, which is the contract-intended behavior. The assertion is stale. I did not change it because the file is not mine and not in WP-149's scope. Owner: Tools/WP-148 or 后端中层 decides.
- Pre-existing and unrelated: `config::path::tests::strip_makes_verbatim_prefix_compatible` fails under `--lib config::`. path.rs is untouched.
- Earlier, a lib-test compile failed on `indexing_wait_message` in src/tools/code_search.rs (another agent's in-progress edit). It resolved on retry.

## Format
- rustfmt (edition 2024) ran only on seed.rs, tools.rs and settings_writer.rs. My global_db/mod.rs edit is 2 small lines and was left to the check.
- `cargo fmt --check`: no diffs in my files. The only diffs are in src/tool/availability.rs (not mine, untouched).

No commit or push.
