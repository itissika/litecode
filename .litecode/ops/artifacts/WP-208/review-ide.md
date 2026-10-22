# WP-208 IDE 基座域 · 核心链路人工核对（基线 3acfe31）

范围：src/lsp、src/workspace、src/optional、src/ide_base.rs、src/platform_knobs.rs 的 #[cfg(test)]；tests/optional_lsp、optional_p0、workspace_api、workspace_path_{consumer,death_list,lap}、workspace_process_contract。合计 364 个测试。
方法：用 `git show 3acfe31:` 读断言来判断，不看测试名下结论；没跑 cargo。
记号：[win] 表示 `#[cfg(windows)]`，Linux 上不跑；[skip?] 表示环境不满足时静默 return。

## 1. 核心链路（18 条）

**L1 Agent/LSP/人工 API 路径解析边界**：相对路径带 `..` 时所有模式都拒绝；Safe 和 LSP 拒绝工作区外的绝对路径，All 放行；人工 API 不接受绝对路径。
- workspace/tool_path.rs::relative_traversal_is_rejected_in_all_modes：All 模式下 `../outside` 返回 Err(RelativeTraversal)
- tool_path.rs::all_allows_and_safe_rejects_external_absolute_path：外部临时文件 All=Ok，Safe=OutsideWorkspace
- tool_path.rs::unix_absolute_outside_workspace_safe_rejected：`/etc/passwd` Safe 拒绝、All 放行（unix）
- tool_path.rs::human_relative_rejects_absolute_paths：resolve_human_relative 遇到绝对路径返回 OutsideWorkspace
- tool_path.rs::agent_and_lsp_share_safe_scope：resolve_lsp_workspace 拒绝外部绝对路径，即 LSP 永远不走 All
- tool_path.rs::raw_path_outside_workspace_matches_resolved_boundary：`../outside` 判为 outside（失败一律当越界），权限匹配用的是这一个函数
- workspace/agent_path.rs::relative_traversal_is_rejected：分类层识别 `..`

**L2 symlink 逃逸（目标文件还不存在时）**：Safe 模式按"最深的已存在祖先"做 canonicalize，经 symlink 目录写新文件一律拒绝。
- tool_path.rs::symlinked_dir_escape_rejected_for_new_file：`link/newfile.txt` 返回 OutsideWorkspace
- tool_path.rs::symlinked_subdir_escape_rejected_for_new_file：嵌套一层 `sub/link/...` 的情形
- tool_path.rs::symlinked_dir_escape_rejected_for_new_file_windows：[win][skip?] 没有建 symlink 的权限时 eprintln 后 return，不能当可靠守护

**L3 Windows 下的 Unix 风格路径**：`/etc/passwd` 不能被误判成相对路径或 traversal，要么映射到 Git for Windows 的路径，要么明确报错。
- agent_path.rs::unix_style_on_windows_not_traversal [win]
- tool_path.rs::unix_style_passwd_not_relative_traversal [win]：断言结果是 Ok(含 etc) 或 UnixStyleOnWindows
- agent_path.rs::msys_drive_mapping [win]：`/c/Users/foo` 映射为 `C:\Users\foo`

**L4 人工 HTTP 和 git 接口越界返回 403**：Sandbox::resolve 只要遇到 traversal、外部或 Unix 风格路径，统一返回 Escape，HTTP 层映射成 403。
- workspace/sandbox.rs::rejects_parent_dir_escape
- tests/workspace_api.rs::workspace_rejects_path_escape：GET file `../secret.txt` 返回 403
- workspace_api.rs::workspace_git_stage_commit_and_rejects_escape：stage `../secret.txt` 返回 403
- workspace_api.rs::workspace_git_show_head_index_and_rejects：show `../secret.txt` 返回 403，非法 rev 返回 400
- workspace_api.rs::workspace_preview_reads_and_open_guards：open `../secret.txt` 返回 403，打开目录返回 400
- workspace/git.rs::path_escape_is_rejected：stage 越界返回 Escape 或 PathNotAllowed，`restore .` 被拒

**L5 复制 symlink 不外泄**：复制或保留 symlink 时，被指向的对象要过沙箱；指向外部的 symlink 拒绝复制，也不留半成品。
- workspace/service.rs::refuses_symlink_copy_escaping_workspace：返回 Sandbox(Escape)，out.txt 不存在，symlink_metadata 也查不到
- service.rs::preserves_symlink_top_level / preserves_symlink_nested：复制结果仍是 symlink，不会被展开成内容

**L6 引用/预览不泄露文件内容，越界引用被隐藏**
- workspace/citations.rs::resolve_reports_exists_line_symbol_and_hides_escapes
- workspace_api.rs::workspace_citations_resolve_without_file_bodies：响应里不含 LEAK_LINES_BODY 和 SECRET_CITATION_BODY，`../` 引用不暴露内容
- workspace/sqlite_preview.rs::rejects_unknown_and_injected_table_names：`items; DROP TABLE items` 返回 UnknownTable，执行后行数仍为 2

**L7 LAP 统一形态（verbatim、UNC、canonicalize 白名单）**：所有根路径和比较都用 LAP 形态；canonicalize 只允许出现在 config/path.rs。
- tests/workspace_path_death_list.rs 下 3 个（death_list_no_bare_canonicalize_outside_path_module、death_list_src_canonicalize_whitelist_is_only_path_rs、death_list_web_no_silent_verbatim_strip）：扫描源码做静态守护
- tests/workspace_path_lap.rs::boot_sandbox_lsp_os_probe_roots_are_identical_lap：boot、Sandbox、LspHub、URI 四处的根完全相同，URI 里不含 `?/`
- workspace_path_lap.rs::strip_verbatim_unc_keeps_unc_prefix、windows_raw_canonicalize_is_verbatim_then_strips_to_lap [win]、snapshot_id_stable_across_verbatim_and_lap_input
- workspace_path_lap.rs::rel_path_ctx_matches_rel_path_under_and_rejects_outside；workspace/filter/path.rs::rel_path_ctx_rejects_outside、cheap_rel_under_rejects_outside、cheap_rel_under_strips_verbatim_prefix [win]
- tests/workspace_path_consumer.rs::rel_path_under_accepts_raw_canonicalize_form、windows_verbatim_walk_path_relativizes [win]
- sandbox.rs::root_is_lap_not_verbatim

**L8 LSP 输出路径**：工作区内的路径显示为相对路径，工作区外保留绝对路径；display_path 只做文本前缀剥离，不解析 symlink。
- lsp/format.rs::location_inside_workspace_is_relative、location_outside_workspace_stays_absolute、workspace_symbol_paths_relative_inside_absolute_outside、display_path_does_not_resolve_symlinks、display_path_tries_roots_in_order

**L9 LSP URI 与盘符**：file URI 去掉 verbatim 前缀、盘符不区分大小写、能还原被破坏的 verbatim 形态。
- lsp/mod.rs::file_to_uri_strips_verbatim_prefix、uri_to_path_recovers_mangled_verbatim_file_uri、normalize_windows_verbatim_file_uri、publish_diagnostics_uri_matches_drive_case、publish_diagnostics_uri_matches_mangled_verbatim

**L10 诊断时效（不回显过时的错误）**
- lsp/conn.rs 下：diagnostics_are_current_rejects_stale_cache、cache_covers_sync_requires_matching_version、wait_ignores_stale_publish_and_keeps_waiting_for_current、wait_returns_current_errors_without_blocking、wait_returns_current_clean_cache_without_blocking、unversioned_publish_after_change_is_silence_not_stale_errors、edit_fix_must_not_echo_previous_round_error
- tests/optional_lsp.rs（mock LS）下：write_without_lsp_warm_never_appends_diagnostics、write_with_warm_lsp_but_no_errors_stays_clean、write_with_warm_lsp_appends_only_when_errors_arrive、write_ignores_slow_diagnostics_past_budget、write_ignores_warn_only_diagnostics、edit_fix_must_not_echo_previous_lsp_error

**L11 ContentModified 处理**：只认错误码 -32801，最多取消一次；透传型 RPC 保留原始错误。
- conn.rs 下：is_content_modified_matches_only_32801、content_modified_plain_request_returns_empty_once、content_modified_semantic_tokens_is_cancelled_once、content_modified_resolve_is_cancelled_once、content_modified_passthrough_rpc_id_keeps_raw_error

**L12 LSP 进程生命周期**：先 sweep 再 spawn；hub 被 drop 时杀掉子进程；请求超时不杀进程；重启有预算。
- lsp/sweep_gate.rs 全部 4 个；lsp/mod.rs::spawn_ls_waits_for_sweep_gate
- optional_lsp.rs::hub_drop_kills_spawned_server_process、request_timeout_does_not_kill_process、idle_reclaim_during_diagnostics_does_not_panic
- lsp/hub.rs::restart_budget_resets_after_cooldown；conn.rs::dispatch_out_of_order_responses；optional_lsp.rs::overlapping_hovers_match_out_of_order_ids

**L13 冷启动与 hub 激活**：probe 推迟到第一次真正需要时才跑；probe 失败会让 hub 失活并报出原始错误；索引未就绪时报 unavailable。
- lsp/mod.rs 下：deferred_activate_runs_no_probe_until_first_need、probe_runs_once_per_activation、probe_failure_deactivates_hub_and_reports_original_error、restart_after_failed_probe_says_hub_not_active、coverage_requires_active_and_configured_command
- lsp/server.rs 下：index_not_ready_error_maps_to_unavailable_wording、diagnostics_gate_table、nav_empty_symbols_by_array_len

**L14 项目根选择**
- lsp/project_root.rs 下：rust_analyzer_uses_cargo_workspace_not_member_crate、rust_nested_crate_outside_workspace_members_keeps_own_root、monorepo_ts_file_resolves_to_web_subdir
- workspace_path_consumer.rs::project_root_fallback_accepts_verbatim_under_lap_ws

**L15 可选引擎懒构建与 reconcile**
- optional/mod.rs 下：defer_reconcile_builds_no_client、ensure_warm_builds_once_and_reuses、concurrent_first_calls_build_once、failed_build_is_not_cached_and_retries、stopped_engine_is_not_rebuilt、settings_reconcile_rebuilds_immediately、poisoned_state_lock_does_not_cascade_panics
- tests/optional_p0.rs::settings_reload_reconciles_engine_manager、network_core_engines_always_warmup；optional_lsp.rs::engines_json_off_no_lsp_tool

**L16 原子写**：写入时保留 symlink 节点、权限位和硬链接；create-only 模式不覆盖已有文件。
- workspace/file_ops.rs 下：atomic_write_writes_through_symlink、atomic_write_preserves_execute_bit_on_overwrite、atomic_write_preserves_hardlink_inode、atomic_create_fails_if_exists_without_clobber、atomic_write_creates_file_with_content

**L17 工作区进程契约**：不 chdir；同一个根只能加一把锁；agent 的 cwd 取已解析的工作区。
- tests/workspace_process_contract.rs 下：load_workspace_state_sets_runtime_paths_without_chdir、workspace_lock_second_acquire_same_root_fails、agent_runtime_cwd_uses_resolved_workspace_not_process_cwd、tests/workspace_path_consumer.rs::active_paths_fallback_is_lap

**L18 watcher 与 git 环境隔离**
- workspace/watcher.rs 下：paths_outside_root_are_ignored（外部路径的事件返回 None）、atomic_save_overwrite_is_reclassified_as_modified、rescan_flag_and_overflow_request_full_rescan
- workspace/git.rs 下：inherited_git_dir_env_does_not_select_snapshot_repo、git_dir_is_snapshot_detects_shadow_repo、git_commands_skip_optional_index_locks

## 2. 和初稿的差异

### 补（初稿漏掉，共 24 个）
tool_path::human_relative_rejects_absolute_paths、agent_and_lsp_share_safe_scope；agent_path::unix_style_on_windows_not_traversal；sandbox::rejects_parent_dir_escape；service::refuses_symlink_copy_escaping_workspace（唯一守住"已存在的 symlink 指向外部"这一情形的测试）；citations::resolve_reports_exists_line_symbol_and_hides_escapes；sqlite_preview::rejects_unknown_and_injected_table_names（SQL 注入）；git::path_escape_is_rejected；filter/path::cheap_rel_under_strips_verbatim_prefix；workspace_path_lap::windows_raw_canonicalize_is_verbatim_then_strips_to_lap；format::location_outside_workspace_stays_absolute、display_path_does_not_resolve_symlinks；lsp/mod::publish_diagnostics_uri_matches_mangled_verbatim、file_to_uri_strips_verbatim_prefix、spawn_ls_waits_for_sweep_gate、probe_failure_deactivates_hub_and_reports_original_error；conn::cache_covers_sync_requires_matching_version、edit_fix_must_not_echo_previous_round_error、is_content_modified_matches_only_32801；sweep_gate::gate_opens_when_sweep_panics；server::diagnostics_gate_table；optional/mod::defer_reconcile_builds_no_client、failed_build_is_not_cached_and_retries；watcher::paths_outside_root_are_ignored。
理由：这些测试都直接断言了越界拒绝、不泄露内容或时效/生命周期的不变量。初稿是脚本按关键词筛的，因此漏掉了。

### 剔（初稿误判，共 8 个）
- config/path.rs 下 4 个（canon_abs_resolves_symlink 等）：不在本域，应交给 config 域核对。L1、L7 依赖它们，但不由本域负责。
- file_ops::atomic_write_writes_through_symlink：它守的是"写透 symlink、保留节点"，属于写入正确性，不是安全守护（它本身恰恰会跟随 symlink）。已移到 L16，不再算路径安全。
- deps::rustup_link_without_component_is_unavailable：属于安装探测，和冷启动、路径无关，降为回归测试。
- install/github_tests::live_rust_analyzer_release_contains_exact_windows_and_linux_names：依赖网络、实时查询，结果不稳定，不能当守护。
- server::publish_diagnostics_payload_extracts_array：只是解析细节，已被 L10 的测试覆盖，非核心。
- 另外，初稿把 optional_lsp 的全部测试都算成核心。其中 rust_analyzer_smoke、agent_lsp_tool_views_with_rust_analyzer 在没装 RA 时会静默 return [skip?]，应剔除。

## 3. 回归测试（守过具体 bug，依据是断言加注释）
- tool_path::symlinked_dir_escape_rejected_for_new_file / symlinked_subdir_… / …_windows：修复 1.5，词法 fallback 能绕过 is_under，导致新文件写到工作区外
- service::refuses_symlink_copy_escaping_workspace：复制 symlink 时外泄内容，或留下半成品链接
- workspace_path_death_list ×3：裸 canonicalize 产生 verbatim 路径，导致前缀比较失效；web 侧会静默剥掉 verbatim
- workspace_path_lap::boot_sandbox_lsp_os_probe_roots_are_identical_lap：各处的根路径形态不一致
- lsp/mod::publish_diagnostics_uri_matches_drive_case / …_mangled_verbatim / uri_to_path_recovers_mangled_verbatim_file_uri：盘符大小写或 verbatim 形态导致诊断丢失
- conn::unversioned_publish_after_change_is_silence_not_stale_errors、edit_fix_must_not_echo_previous_round_error；optional_lsp::edit_fix_must_not_echo_previous_lsp_error：修好代码后仍回显上一轮的错误
- optional_lsp::get_diagnostics_rpc_does_not_deadlock、sync_then_feedback_on_current_thread_does_not_deadlock：死锁
- optional_lsp::idle_reclaim_during_diagnostics_does_not_panic：回收过程中 panic
- optional_lsp::request_timeout_does_not_kill_process：超时误杀 LS 进程
- optional_lsp::overlapping_hovers_match_out_of_order_ids、conn::dispatch_out_of_order_responses：响应乱序后错配
- sweep_gate::gate_opens_when_sweep_panics；lsp/mod::spawn_ls_waits_for_sweep_gate：sweep 误杀刚 spawn 的 LS，或 sweep panic 后永久阻塞（WP-165）
- optional/mod::defer_reconcile_builds_no_client、poisoned_state_lock_does_not_cascade_panics：启动时提前预热；锁中毒引起连锁 panic
- project_root::rust_analyzer_uses_cargo_workspace_not_member_crate、rust_nested_crate_outside_workspace_members_keeps_own_root：同一 workspace 里多开 RA，或嵌套 crate 选错根
- git::inherited_git_dir_env_does_not_select_snapshot_repo：继承的 GIT_DIR 指向快照仓
- git::git_commands_skip_optional_index_locks：index.lock 冲突
- watcher::atomic_save_overwrite_is_reclassified_as_modified、delete_then_recreate_coalesces_to_modified：原子保存被误报为删除
- service::change_broadcast_is_bounded_and_lagged_receiver_recovers：广播无上界，lag 后无法恢复
- index_policy::search_exclude_does_not_gate_code_index_queue：3ddc643，search_exclude 误拦了代码索引
- npm::npm_is_invoked_as_the_shim_not_via_cmd_exe；typescript::pins_typescript_below_7；github_tests::asset_exact_name_skips_vsix；deps::rustup_link_without_component_is_unavailable、rust_analyzer_symlink_to_rustup_spawns_as_rust_analyzer：安装和探测相关的回归
- workspace_process_contract::workspace_open_http_endpoint_is_gone：已移除的接口不能复活

## 4. 其余测试（非核心，按类别计数，约 230 个）
- filter（exclude/preset/walk/glob/index_policy/workspace_excludes/dirs/gitignore/mod）：约 62
- watcher 事件分类（不含 L18）：约 22
- text_codec（BOM/EOL/span）：11
- tree/glob 列表：12
- lsp install（github_tests 其余部分、adapters、npm）：约 26
- lsp deps 探测：约 12
- lsp/mod 其余（server_map、command_parts、extract_locations、状态序列化等）：约 12
- conn/server/hub 其余协议细节：约 8
- git 解析/日志/暂存功能：约 8
- service/file_ops/sqlite_preview/symbols/citations 功能：约 16
- workspace_api 功能（tree/glob/crud/mkdir/status）：7
- optional_lsp 功能/集成（含 2 个 [skip?]）：约 9
- optional_p0 其余 + exa_mcp：8
- platform_knobs：3（只是目录事实，非核心）；ide_base.rs 没有测试
- project_root 语言 marker 和 language_id：5

## 5. 守护空白（核心行为没有测试）
1. **已存在的 symlink 指向工作区外，再经 Safe 或人工 API 读写**：代码上靠 canon_abs 加 is_under 拦住，但只有新文件（L2）和复制（L5）两种情形有测试，直接读、写、编辑都没有测试。
2. **绝对路径里带 `..`**（如 `<ws>/../x`，在 Safe 模式且目标不存在时）：没有测试。
3. **project_root 往上找祖先目录时越过工作区**：停止条件是 `ancestor == ws`，做的是原始相等比较，不是 LAP 或 is_under 比较。输入是 verbatim 形态或盘符大小写不同时，可能选中工作区外的 marker。没有测试断言"工作区之上的 marker 不会被选中"。
4. **大小写敏感的前缀比较**：Sandbox::rel_path 和 RelPathCtx::rel 用的是 `starts_with`，不是 is_under，没有 Windows 盘符大小写的测试。
5. **敏感路径**（.env、.ssh 等）：本域代码里没有实现，实现在 crate::permission::sensitive，需由 permission 域核对。本域唯一相关的是 L1 的 raw_path_outside_workspace（失败按越界处理）。
6. **Windows 链路只有 [win] 测试**（L3，L7 部分，L2 的 windows 版）：Linux CI 上不跑；symlink 那个还会在没有权限时静默跳过。实际守护强度取决于是否有 Windows CI。
7. **lsp/uri.rs、server_map.rs、paths.rs 没有自己的单测**：URI 部分由 lsp/mod.rs 间接覆盖，paths.rs 没有覆盖。
8. **真实 RA 冒烟**依赖本机环境，CI 上可能从未真正跑过。
