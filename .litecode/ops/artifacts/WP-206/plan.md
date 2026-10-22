目标：修 Code 模式测评 8 项（brief WP-206），run 恢复短命令前台拿结果且并行真并发。
方案（详见 dig-survey.md）：
A Runtime #1+#2 同一 commit：BACKGROUND_EXIT_WINDOW→30s；executor 对 releases_lock_after_spawn 工具把 EffectGuard/WriteLockGuard 移入 ToolExecutionContext.spawn_guard，run 在 cmd.spawn() 成功后 release（Drop 兜底）；run.md 补 30s 与 await 顺序说明。
B Tools #4#5：read 越 EOF resolve 空 lines+“共 N 行”message，翻转测试，read.md；glob.md basename 任意深度说明。
C IDE #3#8：documentSymbol/workspaceSymbol 走 request_nav_with_retry，diagnostics 按 server_ready/fresh reject unavailable；format.rs 用 cheap_rel_under 输出工作区相对路径。
D PTC #6#7：illegal_at 带路径；from_js UndefinedPolicy（入参静默剥除，返回值剥除/数组置 null+head dropped 行封顶）；PRELUDE/schema/charter I10/remediation plan 同步。
顺序：编码 A/B/C/D 文件不重叠可并行；cargo 串行（free≥2GiB，-j 2），顺序 A→C→D→B；SDK fixture+sha 只在最后统一刷新一次并跑 sdk_snapshot。
验收：brief 验收 1-9 全部带测试；相关模块+sdk_snapshot 全绿；cargo 日志入 artifacts/WP-206；push feat/tool-contract-ptc，不合 main。
Q1-Q7 见 dig-survey.md（均带推荐）。
