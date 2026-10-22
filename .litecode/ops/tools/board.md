# LiteCode Ops Tools Board

> 聚合面板：最近更新 · 使用教程 · 运行时版本感知
> 每次工具输出第一行均为版本头：`LITECODE_OPS tools_version=N tool=... tool_version=...`

## 当前版本

- `tools_version`: **21**
- registry: `.litecode/ops/tools/registry.json`
- 查询: `node .litecode/ops/tools/tools_version.mjs`

<!-- WP_STATUS:BEGIN (auto, wp_gate/tick 生成，勿手改) -->
## WP 状态

> 自动生成于 2026-10-22T03:53:53.582Z，来源 `.litecode/ops/board.json`

| id | 标题 | status | owner | reviewer |
|---|---|---|---|---|
| WP-001 | 冻结团队框架 v0：域+面板+工具门禁+冷启动稿 | done | gm | andy |
| WP-002 | （断点后）装填 Runtime/上下文/IDE/需求官/工具集管家 | done | gm | andy |
| WP-003 | 管理团队冷启动对齐（事件+tick+职责移交） | done | gm | andy |
| WP-004 | 修 ISSUE-010：copy_path 顶层 symlink 跟随后复制目标 | cancelled | ide | gm |
| WP-005 | 修 ISSUE-008：PermissionModal 过期 justify-start 断言 | done | frontend | gm |
| WP-006 | 修 ISSUE-005：repair_agent_models_on_boot 越界 rusqlite | done | runtime | gm |
| WP-007 | 修 ISSUE-018：CompactionFailed 文案与实现一致 | done | runtime | gm |
| WP-008 | 可选加固 ISSUE-017：commit_mixed 半写入 | done | runtime | gm |
| WP-009 | 可选加固 ISSUE-021：webfetch DNS 重绑定窗口 | done | tools | gm |
| WP-010 | 前端团队扩编论证：按准入规矩钉节点清单并反推 bot 数量 | done | frontend | andy |
| WP-011 | 深挖 code/session search 易用性失败线；LSP 仅易用性探索（下载/拉起/保活） | done | ide | andy |
| WP-012 | 主链测试体系打稳：裁剪冗余/过时、补关键链路、排查假绿 | done | demand_officer | andy |
| WP-013 | 知识易用性工具增强：引用导航/被引/脚手架（非仅 kb_check） | done | tools_steward | gm |
| WP-014 | 补测：compact 失败三口径（auto 停 turn / 失败还原 / manual 可重试） | done | runtime | gm |
| WP-015 | 主链测试打稳跟进包（g2 门改写、debug-fold、瘦包门、Tools WS/websearch） | done | demand_officer | gm |
| WP-016 | 冻结：检索引擎产品行为（起即可搜 / 刷不挡搜 / 空结果才谈库旧） | cancelled | gm | andy |
| WP-017 | 稳域钉稳：必过测试清单+交叉审+实跑绿表（禁碰引擎产品/symlink复制） | done | demand_officer | gm |
| WP-018 | 找不稳实现 + 裁冗余测试（前后端中层，只列不删待 GM 审） | cancelled | backend_mid | gm |
| WP-019 | 嵌套 WP 工具（顶层，Andy 批） | done | gm | gm |
| WP-020 | └ [WP-019 子包] 嵌套 WP 配套工具：管家先填方案交 GM 审 | done | tools_steward | gm |
| WP-021 | 用新流程重做：找不稳实现 + 裁冗余测试 | done | gm | andy |
| WP-022 | └ 后端：找不稳实现 + 列测试裁剪清单 | done | backend_mid | gm |
| WP-024 | 　└ WP-022/Session：不稳实现+测试裁剪 | done | session | backend_mid |
| WP-025 | 　└ WP-022/Runtime：不稳实现+测试裁剪 | done | runtime | backend_mid |
| WP-026 | 　└ WP-022/上下文：不稳实现+测试裁剪 | done | context | backend_mid |
| WP-027 | 　└ WP-022/Tools：不稳实现+测试裁剪 | done | tools | backend_mid |
| WP-028 | 　└ WP-022/IDE 基座：不稳实现+测试裁剪 | done | ide | backend_mid |
| WP-023 | └ 前端：找不稳实现 + 列测试裁剪清单 | done | frontend_mid | gm |
| WP-029 | 　└ WP-023 web：找不稳实现+R2+测试裁剪清单 | done | frontend | frontend_mid |
| WP-030 | 　└ WP-023 desktop：找不稳实现+测试裁剪清单 | done | frontend_desktop | frontend_mid |
| WP-031 | ops 工具加固：超时去重 + 事件持久化 + issue 编号加锁 + rustup 默认 | done | gm | andy |
| WP-032 | └ ops 工具加固（管家） | done | tools_steward | gm |
| WP-033 | 按序修稳已定问题（权限卡/断线/进程生命周期/下载/压缩真相/workspace） | done | gm | andy |
| WP-034 | └ web：权限卡队列+单卡UI + 断线立刻失败在途请求 | done | frontend_mid | gm |
| WP-037 | 　└ WP-034 web：权限队列+单卡 + 断线 reject pendingRpc | done | frontend | frontend_mid |
| WP-035 | └ desktop：自动重连/重启 + 进程单飞 + 下载体验 | done | frontend_mid | gm |
| WP-041 | 　└ WP-035 desktop：隧道/sidecar 自动恢复+单飞 → 下载体验 | done | frontend_desktop | frontend_mid |
| WP-036 | └ 后端：进程生命周期统一 + 压缩DB真相 + workspace 覆盖 | done | backend_mid | gm |
| WP-038 | 　└ WP-036/Tools：统一 custom+MCP 进程生命周期 | done | tools | backend_mid |
| WP-039 | 　└ WP-036/上下文：压缩以 Session DB 为唯一真相 | done | context | backend_mid |
| WP-040 | 　└ WP-036/IDE：workspace 并发覆盖 + 跨盘移动 | done | ide | backend_mid |
| WP-042 | 测试：先覆盖后裁剪（核心流程 → 合理量级） | done | gm | andy |
| WP-043 | └ 后端测试：核心流程覆盖核对 + 裁剪裁决 | done | backend_mid | gm |
| WP-047 | 　└ WP-043/Session：核心覆盖核对 + 测试裁剪提案 | done | session | backend_mid |
| WP-048 | 　└ WP-043/Runtime：核心覆盖核对 + 测试裁剪提案 | done | runtime | backend_mid |
| WP-049 | 　└ WP-043/上下文：核心覆盖核对 + 测试裁剪提案 | done | context | backend_mid |
| WP-050 | 　└ WP-043/Tools：核心覆盖核对 + 测试裁剪提案 | done | tools | backend_mid |
| WP-051 | 　└ WP-043/IDE：核心覆盖核对 + 测试裁剪提案 | done | ide | backend_mid |
| WP-044 | └ 前端测试：核心流程覆盖核对 + 裁剪裁决 | done | frontend_mid | gm |
| WP-045 | 　└ WP-044 web：核心覆盖核对 + 测试裁剪提案 | done | frontend | frontend_mid |
| WP-046 | 　└ WP-044 desktop：核心覆盖核对 + 测试裁剪提案 | done | frontend_desktop | frontend_mid |
| WP-052 | 产品缺陷级钉稳（文件变更不丢 + MCP 超时/按 id 匹配） | done | gm | andy |
| WP-053 | └ 后端：组织修 ISSUE-032 文件变更丢失 + ISSUE-034 MCP 超时/id 匹配 | done | backend_mid | gm |
| WP-054 | 　└ WP-053/Runtime：ISSUE-032 take_changes 失败不丢通知 | done | runtime | backend_mid |
| WP-055 | 　└ WP-053/Tools：ISSUE-034 MCP stdio 超时 + id 匹配 | done | tools | backend_mid |
| WP-056 | 　└ WP-053/上下文：FileTracker::mark_dirty 供 ISSUE-032 回灌 | done | context | backend_mid |
| WP-057 | 检索引擎产品口径落地 | done | gm | andy |
| WP-058 | └ 后端：引擎 Warming 可见性/重建刷新/session 语义 | done | backend_mid | gm |
| WP-060 | 　└ WP-058/IDE：引擎 Warming 桶 + 废 DirtyTooLarge + 可搜语义 + sync% 数据源 | done | ide | backend_mid |
| WP-061 | 　└ WP-058/Tools：工具可见性 + 废暖机长等 + 结果头 sync% + session 降级表面 | done | tools | backend_mid |
| WP-059 | └ 前端：工具结果视图同步百分比 | done | frontend_mid | gm |
| WP-062 | 　└ WP-059 web：工具结果视图开头同步百分比 | done | frontend | frontend_mid |
| WP-063 | 合包：保链接复制 + 类型化压缩失败 + 已定小缺陷 | done | gm | andy |
| WP-064 | └ 后端：组织 IDE(010保链接) + Runtime(036) + 门禁(041) | done | backend_mid | gm |
| WP-067 | 　└ WP-064/IDE：copy_path 保链接（ISSUE-010） | done | ide | backend_mid |
| WP-068 | 　└ WP-064/Runtime：类型化压缩失败到 TurnCompleted（ISSUE-036 emit） | done | runtime | backend_mid |
| WP-069 | 　└ WP-064/Session：turn_error 类型化消费 + death_list 假红（ISSUE-036/041） | done | session | backend_mid |
| WP-065 | └ 前端：组织 desktop(015 remap-path-prefix / 构建路径卫生) | done | frontend_mid | gm |
| WP-066 | 　└ WP-065 desktop：release remap-path-prefix / 构建路径卫生 | done | frontend_desktop | frontend_mid |
| WP-070 | LSP/Tools 收尾小活 | done | gm | andy |
| WP-071 | └ 后端中层：LSP/Tools 收尾协调 | done | backend_mid | gm |
| WP-072 | 　└ WP-071/Tools：ISSUE-042 sessions.db 走 session 层 + optional_lsp:1409 文案裁定 | done | tools | backend_mid |
| WP-073 | 　└ WP-071/IDE：deps 探测 stdin null（ISSUE-043）+ hub -32801 有限重试（ISSUE-044） | done | ide | backend_mid |
| WP-074 | 　└ WP-071/IDE-2：LSP server 以原始路径启动（ISSUE-045） | done | ide | backend_mid |
| WP-075 | 　└ WP-071/Tools-2：ISSUE-047 A — lsp 工具透出 ContentModified 提示 | done | tools | backend_mid |
| WP-076 | 语义检索后台静默刷新 + ask_user 自由输入固定末项 | done | gm | andy |
| WP-077 | └ 后端中层：检索静默刷新 + ask_user 后端/CLI | done | backend_mid | gm |
| WP-079 | 　└ WP-077/IDE：代码语料 30s 静默增量刷新 + 代码/会话计划拆分 | done | ide | backend_mid |
| WP-080 | 　└ WP-077/Tools：ask_user schema 去 allow_free_text + 描述「其他（自己填写）」 | done | tools | backend_mid |
| WP-081 | 　└ WP-077/Session：ask_user 协议恒 free_text + CLI 末项「其他（自己填写）」 | done | session | backend_mid |
| WP-078 | └ 前端中层：AskUserCard 末项自由输入 | done | frontend_mid | gm |
| WP-082 | 　└ WP-078 web：AskUserCard 末项固定自由输入（方案B） | done | frontend | frontend_mid |
| WP-083 | 　└ WP-078 web 补：末项常驻文案+单选选中态 | done | frontend | frontend_mid |
| WP-084 | 调研：subagent session 操作缺口 | done | gm | andy |
| WP-085 | └ 后端中层：subagent session 操作缺口调研 | done | backend_mid | gm |
| WP-086 | subagent session 操作补全（按 Andy 收敛口径） | done | gm | andy |
| WP-087 | └ 后端中层：subagent 操作补全后端 | done | backend_mid | gm |
| WP-090 | 　└ WP-087/Runtime：cancel 如实原语 + 停/删通知不唤醒 + Lagged 补投 + 子 deny sink | done | runtime | backend_mid |
| WP-091 | 　└ WP-087/Session：拒子 run/enqueue + cancel 回包如实 + 删除清理/descendant_running + 墓碑 + 原始状态 | done | session | backend_mid |
| WP-092 | 　└ WP-087/Tools：subagent_send 入 pending + stop 原子 + 新 subagent_delete + 权限描述 | done | tools | backend_mid |
| WP-088 | └ 前端中层：子会话 停/删 入口与展示 | done | frontend_mid | gm |
| WP-089 | 　└ WP-088 web：子会话 停止/删除 入口与删除后展示/重连 | done | frontend | frontend_mid |
| WP-093 | 　└ WP-088 web 补：listSessions 进行中再请求则结束后补拉一次 | done | frontend | frontend_mid |
| WP-094 | subagent 发送模式与结算收口 | done | gm | andy |
| WP-095 | └ WP-094 后端中层：发送模式/结算/停止不唤醒/来源字段/测试/知识库 | done | backend_mid | gm |
| WP-098 | 　└ WP-095/Runtime：send 三模式原语 + 最新结算邮箱 + 停止不唤醒 + 通知 by | done | runtime | backend_mid |
| WP-099 | 　└ WP-095/Session：用户停/删写 by=user + 协议 by 字段 + 子只读保持 | done | session | backend_mid |
| WP-100 | 　└ WP-095/Tools：send mode 参数 + stop/delete 不给自己发通知 + wait 最新 + 描述/知识库 | done | tools | backend_mid |
| WP-096 | └ WP-094 前端中层：侧栏用户停止标记 | done | frontend_mid | gm |
| WP-097 | 　└ WP-096 web：Workers 名单 用户停止标记 | done | frontend | frontend_mid |
| WP-101 | 向量检索引擎开销与体验调研 | done | gm | andy |
| WP-102 | └ 引擎开销与体验调研执行 | done | backend_mid | gm |
| WP-103 | 　└ WP-102/IDE：引擎内存与嵌入耗时实测 + 性能杠杆 + 断点续嵌调研 | done | ide | backend_mid |
| WP-104 | 　└ WP-102/Session：会话语料构成只读统计 | done | session | backend_mid |
| WP-105 | provider 供应商与模型更新调研 | done | gm | andy |
| WP-106 | └ provider 调研执行 | done | backend_mid | gm |
| WP-107 | 　└ WP-106/A 一方直连：openai/deepseek/mimo/zhipu + 两种 codec 对照 | done | provider | backend_mid |
| WP-108 | 　└ WP-106/B 国内云套餐：ark-coding/ark-agent/tencent-token/aliyun-token | done | provider | backend_mid |
| WP-109 | 　└ WP-106/C 聚合/网关：opencode/opencode-go/commandcode + Q4 新供应商候选 | done | provider | backend_mid |
| WP-110 | provider 补模型/新加两家 + 前端分组 | done | gm | andy |
| WP-111 | └ provider 目录落地 | done | backend_mid | gm |
| WP-113 | 　└ WP-111 目录落地执行 | done | provider | backend_mid |
| WP-112 | └ provider 列表分组 | done | frontend_mid | gm |
| WP-114 | 　└ WP-112 web：provider 列表 plan/api 分组 | done | frontend | frontend_mid |
| WP-115 | 语义检索：语料本质与稠密/稀疏对齐续测 | cancelled | backend_mid | gm |
| WP-116 | 语义检索续测：语料本质与稠密/稀疏对齐 | done | gm | andy |
| WP-117 | └ WP-116 续测执行 | done | backend_mid | gm |
| WP-118 | 　└ WP-117/IDE：稠密vs稀疏对齐与可删开销 | done | ide | backend_mid |
| WP-119 | 　└ WP-117/Session：三态口径语料重切统计 | done | session | backend_mid |
| WP-120 | 语义检索：数据源白名单·切块·算法定口径 | done | gm | andy |
| WP-121 | └ WP-120 定口径执行 | done | backend_mid | gm |
| WP-122 | 　└ WP-121/IDE：白名单·切块·算法小样本实测 | done | ide | backend_mid |
| WP-123 | 　└ WP-121/Session：白名单语料切片与小样本 query 集 | done | session | backend_mid |
| WP-124 | 语义检索：真引擎算法与切块定口径 | done | gm | andy |
| WP-125 | └ WP-124 真引擎续测执行 | done | backend_mid | gm |
| WP-126 | 　└ WP-125/Session：可对齐金标 query 集（白名单语料） | done | session | backend_mid |
| WP-127 | 　└ WP-125/IDE：真引擎 ANN+RRF 与切块小样本 | done | ide | backend_mid |
| WP-128 | 语义检索落地：白名单准入 | done | gm | andy |
| WP-129 | └ WP-128 白名单准入执行 | done | backend_mid | gm |
| WP-130 | 　└ WP-129/IDE：白名单准入落地（稠密三态 / 稀疏+tool_call） | done | ide | backend_mid |
| WP-131 | 引擎进度条：code+session 合并状态 | done | gm | andy |
| WP-132 | └ 后端：code+session 统一 chunk 进度上报 | done | backend_mid | gm |
| WP-134 | 　└ 引擎进度：code 侧 + 两库合并 | done | ide | backend_mid |
| WP-135 | 　└ 引擎进度：session 侧上报 | done | session | backend_mid |
| WP-137 | 　└ 引擎进度：tool sync% 改用新 total | done | tools | backend_mid |
| WP-133 | └ 前端：进度条按合并状态呈现 | done | frontend_mid | gm |
| WP-136 | 　└ WP-133 web：引擎进度条 code+session 合并 | done | frontend | frontend_mid |
| WP-138 | 　└ WP-133 web：sync_pct 首行新契约解析 | done | frontend | frontend_mid |
| WP-142 | 　└ WP-133 web：retrieval 待处理改看 files_pending | done | frontend | frontend_mid |
| WP-139 | code 索引刷新：定期扫盘+持久化元数据 | done | gm | andy |
| WP-140 | └ 后端：扫盘对账+印章持久化落地 | done | backend_mid | gm |
| WP-141 | 　└ IDE：code 定期扫盘对账+印章持久化 | done | ide | backend_mid |
| WP-143 | 　└ IDE：code 首建/重建并入增量 + 按批存盘 + 原子写 | done | ide | backend_mid |
| WP-144 | 　└ Session：session 首建/重建按批检查点续跑 | done | session | backend_mid |
| WP-145 | 工具可见性只看 bind：引擎状态只体现在调用结果 | done | gm | andy |
| WP-146 | └ 后端：code_search 可见性只看 bind，引擎状态进调用结果 | done | backend_mid | gm |
| WP-148 | 　└ Tools：code_search 可见性只看 bind + 非就绪返回契约 | done | tools | backend_mid |
| WP-149 | 　└ Runtime：主 agent 默认绑定 code_search（seed+只补缺行迁移） | done | runtime | backend_mid |
| WP-147 | └ 前端：设置页 bind 只显示 desire | done | frontend_mid | gm |
| WP-150 | 　└ WP-147 web：code_search bind 开关只显示库存期望 | done | frontend | frontend_mid |
| WP-151 | 前端瘦身调研（只读，为多语言与介绍文档铺路） | done | frontend_mid | gm |
| WP-152 | └ WP-151 web：前端瘦身只读调研 | done | frontend | frontend_mid |
| WP-153 | └ WP-151 desktop：前端瘦身只读调研 | done | frontend_desktop | frontend_mid |
| WP-154 | 打开工作区延迟调研（只读） | done | backend_mid | gm |
| WP-155 | └ WP-154A 后端工作区打开链路耗时（只读） | done | ide | backend_mid |
| WP-156 | └ WP-154B serve 握手与会话加载耗时（只读） | done | session | backend_mid |
| WP-157 | └ WP-154C 前端握手到可交互耗时（只读） | done | frontend_mid | backend_mid |
| WP-158 | 前端瘦身（按 WP-151 清单与 GM 裁定） | done | frontend_mid | gm |
| WP-159 | └ WP-158 B1 web：低风险删改与依赖声明 | done | frontend | frontend_mid |
| WP-160 | └ WP-158 B1 desktop：低风险删改 | done | frontend_desktop | frontend_mid |
| WP-161 | └ WP-158 B2 desktop：跨端桥接契约 | done | frontend_desktop | frontend_mid |
| WP-162 | └ WP-158 B2 web：改用契约与样式收拢 | done | frontend | frontend_mid |
| WP-163 | └ WP-158 B3 web：重复组件合并与断开态 | done | frontend | frontend_mid |
| WP-164 | └ WP-158 B3 desktop：重复实现合并与跳过令牌 | done | frontend_desktop | frontend_mid |
| WP-165 | 打开工作区启动排位整改 | done | backend_mid | gm |
| WP-166 | └ 启动排位整改·Session 域 | done | session | backend_mid |
| WP-167 | └ 启动排位整改·IDE 域 | done | ide | backend_mid |
| WP-168 | └ 启动排位整改·壳与前端 | done | frontend_mid | backend_mid |
| WP-169 | 　└ 启动排位整改·desktop 壳 | done | frontend_desktop | frontend_mid |
| WP-170 | 　└ 启动排位整改·web 前端 | done | frontend | frontend_mid |
| WP-171 | 新电脑 litecode 环境初始化 | done | tools_steward | gm |
| WP-172 | 重型校验迁到 GitHub Actions | done | tools_steward | gm |
| WP-173 | wp_watch 开机自启常驻（事件提醒） | done | tools_steward | gm |
| WP-174 | wp_watch：禁止回放已关闭包的历史事件 | done | tools_steward | gm |
| WP-175 | 修 ISSUE-064：Command Code / Luna 输出不崩 agent loop | done | backend_mid | gm |
| WP-178 | └ WP-175-a：provider 诊断 H1（Responses SSE / wire+fixture） | done | provider | backend_mid |
| WP-176 | 修 ISSUE-065：ask_user 卡不得未操作即跳过 | done | frontend_mid | gm |
| WP-177 | └ WP-176 web：ask_user 残卡守卫与测试 | done | frontend | frontend_mid |
| WP-179 | 方案：ask_user/权限类调用的会话等待态持久化（不实现） | done | backend_mid | gm |
| WP-180 | 方案：Luna/工具管线崩溃的健壮性与可观测性（不实现） | done | backend_mid | gm |
| WP-181 | 实现：waiting_for_user + 权限类落盘（自 WP-179） | done | backend_mid | gm |
| WP-186 | └ WP-181-a：session 等待态落盘 + pending_permissions + 断线≠Deny | done | session | backend_mid |
| WP-187 | └ WP-181-b：runtime PhasePermissionSink / waiting_for_user phase 对齐 | done | runtime | backend_mid |
| WP-182 | 实现：FE 消费 pending_permissions / waiting_for_user（自 WP-179） | done | frontend_mid | gm |
| WP-184 | └ WP-182 web：pending_permissions hydrate 与出卡 | done | frontend | frontend_mid |
| WP-185 | └ WP-182 desktop：同契约无分叉 | done | frontend_desktop | frontend_mid |
| WP-183 | 实现：崩溃/回合失败可观测性 Obs（自 WP-180） | done | backend_mid | gm |
| WP-188 | └ WP-183-a：runtime Obs-1/2 可观测性落地 | done | runtime | backend_mid |
| WP-189 | 分析：同 session 切 codec 稳定性（含 session/ctx） | done | backend_mid | gm |
| WP-190 | 拣回：ops 易用度（原 WP-189 org/charter/request/zh） | done | tools_steward | gm |
| WP-191 | 实现：切模型/请求关键要素观测与显式告警 | done | backend_mid | gm |
| WP-192 | └ FE: WP-191 toast SnapshotNotice + busy set-model rollback | done | frontend_mid | frontend_mid |
| WP-193 | 审阅：PTC/code 模式对 agent 的可理解与易用 | done | backend_mid | gm |
| WP-194 | 实现：PTC 描述/SDK 文案对齐裸 data | done | backend_mid | gm |
| WP-195 | 调研：bash 系列是否进 Code 模式 | done | backend_mid | gm |
| WP-196 | 实现：bash 家族最低风险切口进 Code（防退化） | done | backend_mid | gm |
| WP-197 | 实现：shell run/wait/kill 精简契约（硬清历史） | done | backend_mid | gm |
| WP-198 | 实现：edit/apply_edits 精简契约（硬清） | done | backend_mid | gm |
| WP-199 | 实现：grep/glob/read 精简（硬清冗余） | done | backend_mid | gm |
| WP-200 | 实现：run_code 管线硬清冗余分层 | done | backend_mid | gm |
| WP-201 | 实现：run_code 人看聚合 data 通路（无前端） | done | backend_mid | gm |
| WP-202 | 实现：LSP 拆多工具+行列对齐并进 Code | done | backend_mid | gm |
| WP-203 | 实现：webfetch/websearch 进 Code（软失败） | done | backend_mid | gm |
| WP-204 | 分析：工具改造后提示词文本面查漏补缺 | done | backend_mid | gm |
| WP-205 | 修复：session_search 溢出文件/虚拟 read 测试失败 | done | backend_mid | gm |
| WP-206 | Code 模式测评修复：run 前台窗/并发、LSP 冷启动、read EOF、边界序列化、路径一致 | done | backend_mid | gm |
| WP-207 | 测试瘦身：删迁移期 gate/过时测，修 stack overflow 分类，全量 cargo test 绿 | done | backend_mid | gm |
| WP-208 | 测试大幅瘦身：默认删除，只守关键模块核心链路（目标 ≤~1000） | done | backend_mid | gm |
| WP-209 | run_code 模型视图整形：表格化/紧凑 JSON + token 预算 + 成功省略 logs | done | backend_mid | gm |
<!-- WP_STATUS:END -->

## 最近更新

| 时间 | 变更 |
|---|---|
| 2026-10-15 | v22（WP-174）：wp_watch 冷启动/接管跳过已关闭或状态已变的 approval/acceptance/timeout outbox 重放（eventStillDeliverable + skipped 记入 state）；ops_harden 组 11 回归（工具集管家）|
| 2026-10-15 | v21（WP-173）：wp_watch 开机自启常驻。新增 ops.wp_watch_service（install/prove/test-callback）；systemd user unit（Restart=always）优先，无 systemctl 时 fallback svc + ~/.profile 钩子；webhook 只写仓库外 `~/.config/litecode/wp-watch.env`（0600），永不打印/入库；去掉「靠定时 tick 提醒」——停滞/升级事件只由常驻 watcher 发出；按 bot 分发回调仅留 resolveWebhook 设计口（工具集管家）|
| 2026-10-15 | v21：合并 gm/wp165-local 与 ops/team-framework 两条各自的「v20」（一条是 WP-171/172：LC_ROOT/init_env/test_failures；一条是 registry_verify --quick/g++ 用户态/cargo_locked 自动校验），tools_version 取 21；registry 保留两边条目（cargo_env v2 含 LC_ROOT 说明）（工具集管家，合并）|
| 2026-10-15 | v20：registry_verify 加 --quick 与 g++ 检查（debs 离线重建用户态 g++）；cargo_locked 取锁前自动 quick verify/自修；cargo_env 自动用 gxx-bin、set -u 安全；文档：g++ 两条路径、重启清单、registry 丢 target/build 目录的取证（工具集管家）|
| 2026-10-15 | v19：watcher 韧性（事故：快照回滚后 watcher 消失 7 小时，tick 无人驱动）。wp_watch v4：ensure / test-callback / watchdog-start|stop，start 等新心跳；ops.wp v6：写操作顺带 softEnsureWatcher；测试 ops_harden 组 10（工具集管家）|
| 2026-10-13 | v17：validate_board v9 新规则「每个 WP.domain 必须在 board.domains」；ops.wp v5 已封包可 `set --field domain` 重新归类（其他字段仍拒）；WP-070/071→ide-substrate、WP-082/083/089/093/097→client-ux；wp.test 组 7 / ops_harden 组 9 老包数改为从 board 动态计算（>0）并逐包校验（工具集管家）|
| 2026-10-13 | v16：ops.wp v4 `set --field domain`（owner/approver；须为 board.domains 已有 id；history+log+trace；校验失败回滚）；新增 `reassign --owner`（包 approver 或父包 owner；新 owner 须已注册；只改 owner；history+log；回滚）；用它们把 WP-103 domain ide-base→ide-substrate；新 agent provider（model-config 域 provider/模型部分，bot id 记 agents_cold/bot_ids.json）（工具集管家）|
| 2026-10-12 | v15（WP-032）：超时按父子链合并、只叫离卡点最近真正要动手的人（有未封子包的 in_progress 父包不算卡点），matter 去重，已 ack 60 分钟内不重发（老 WP 同）；tick 在 watcher 健康时只报告、watcher 挂了自动拉起并记 team_ops.watcher_restarted；事件加锁 + fsync 原子写，先落 inbox+events/outbox.jsonl 再发，重启排空 outbox，所有告警类型都走 webhook（payload 带 target/type/actor/merged/matter_key）；新增 ops.issue_new（加锁分配 issue 号，独占创建）、issues/.reserved.json（031/006 作废）、validate_all 查 issue 名实/重复；新增 ops.toolchain_check；ops.wp v3 加 set；~/.cargo/bin 换 wrapper、~/.rustup 指向 ops toolchain（备份在 ~/.litecode-toolchain-backup-20261012-184910）（工具集管家）|
| 2026-10-12 | v14：ops.wp v2——方案审批与完工验收分开（submit→approve / start→deliver→close，reject 分方案退回与验收退回）；split 方案只需至少 1 个子包；父包方案未批前子包不能交方案或开工；awaiting_approval 与 review 两种等待都超时升级（顶层止于 gm）；ops.wp.test 入 registry；写明工具需求指挥链（工具集管家，Andy 批）|
| 2026-10-12 | v13：新增 ops.wp（嵌套 WP 生命周期 new/fill/submit/approve/reject/close + tree/summary/history）；schema 加 parent/goal/approver/content/history 与 drafting/awaiting_approval；validate 查 parent 环/封包；wp_gate 对新模型包加批准/子包封包/approver 推 done 门禁；超时扩到 awaiting_approval 并按层级升级（顶层只到 gm）；webhook 加 level/parent/path/target/escalated；「WP 状态」表按父包缩进；老 WP 行为不变（工具集管家，WP-020）|
| 2026-10-12 | v12：新增 ops.kb_nav（按 key 查出边/入边、缺边/无父节点/孤儿）与 ops.kb_scaffold（新节点声明块+domain/上层|父节点/基线脚手架；--write 不覆盖）；selftest 固定夹具+live smoke（工具集管家）|
| 2026-10-12 | v11：新增 ops.wp_watch 常驻 watcher——每 5 分钟复用 tick 超时判定，新的未 ack team_ops.wp_timeout 向 GM 回调 POST 一次（Bearer + HMAC，2xx 才算送达，跨重启不重发，失败退避重试；env 未配只记日志）；supervisor 崩溃自动拉起、心跳/日志/状态落 `.litecode/ops/.tmp`（已 gitignore）；tick 每轮查心跳，超 15 分钟 emit team_ops.watcher_down（工具集管家）|
| 2026-10-12 | v10：kb_check 不再把 Andy（itissika）提交设定的新节点 enabled 报 new_not_pending，改为 warning new_status_by_andy；未提交或非 Andy 的状态改动仍为 error（工具集管家）|
| 2026-10-12 | v9：基于 WP 状态的超时事件——wp_gate 记 status_entered_at；tick 在 HKT 工作日 9:45–17:45 对 in_progress/review 连续>1h 未封到 done/blocked 的 WP emit team_ops.wp_timeout（可含 reviewer=andy）；无超时则安静（工具集管家）|
| 2026-10-12 | v8：board.md 新增自动生成的「WP 状态」表（id/标题/status/owner/reviewer），wp_gate 写入状态后与 tick 每轮都会刷新（工具集管家）|
| 2026-10-11 | v7：新增 ops.kb_check（知识库校验，复刻 litecode 解析，cases.json 87/87）；validate_all 接入 kb_check（有 error 即失败）；wp_gate 对知识类 WP 推到 review/done 前要求涉及节点无 error（工具集管家）|
| 2026-10-11 | v6：wp_gate 通过后写回 board.json 状态 + 自动 append status_change log + 重跑 validate（失败回滚）；--dry-run 保留只检查（工具集管家）|
| 2026-10-11 | v5：工具一览补 emit/ack；ops.board 版本对齐；emit/ack 补 stdout_header（工具集管家）|
| 2026-10-11 | v4：事件总线 emit/ack；工具维护权移交工具集管家；GM 仅 tick 兜底
| 2026-10-11 | v3：stdout 首行版本头；本 board；team-ops bot 已建且未发首条 |
| 2026-10-11 | v2：validate 允许人类审查员 andy |
| 2026-10-11 | v1：gate / tick / validate / append_log |

## 使用教程（强制）

1. 调用前感知版本：读输出首行或跑 `tools_version.mjs`
2. 推进 WP：`wp_gate.mjs --id WP-xxx --to <status>`（通过即写回 board.json 并记 log；只检查加 `--dry-run`）；done 需产物 + tool_traces
3. 审计：`append_log.mjs --kind --source --summary [--wp]`
4. 看板刷新（可选）：`tick.mjs` 仍可刷新「WP 状态」表并报告超时扫描结果；**v21 起不再把 tick 当定时提醒**——停滞/升级事件由常驻 `wp_watch` 服务发出。watcher 健康时 tick 只报告不 emit。
5. WP 停滞回调（v21）：安装一次 `node .litecode/ops/tools/wp_watch_service.mjs install --wp WP-xxx`（systemd 或 fallback 开机自启 + 崩溃拉起）。日常用 `status` / `prove` / `test-callback`。详见「ops.wp_watch / wp_watch_service 用法」
6. 热更：改完 `tools_version++`，并在本表追加一行；破坏性变更用新 tool id
7. 知识导航 / 脚手架（v12）：摸引用链用 `kb_nav.mjs --key <key>`，列孤儿加 `--orphans`；起草新节点用 `kb_scaffold.mjs --key --domain`（默认只打印，加 `--write` 才落盘且不覆盖）。详见下方用法
8. 知识库自查（v7）：改/建知识节点后跑 `kb_check.mjs`（只看一个节点加 `--node <key>`），error 必须清零；`validate_all` 会同样拦截。详见下方「ops.kb_check 用法」

## 工具一览

| id | 命令 |
|---|---|
| ops.tools_version | `node .litecode/ops/tools/tools_version.mjs` |
| ops.validate_board | `node .litecode/ops/tools/validate_all.mjs [--wp WP-xxx]` |
| ops.append_log | `node .litecode/ops/tools/append_log.mjs --kind --source --summary [--evidence] [--wp]` |
| ops.wp_gate | `node .litecode/ops/tools/wp_gate.mjs --id WP-xxx --to <status> [--dry-run] [--source <agent_id>] [--kb-nodes k1,k2]` |
| ops.tick | `node .litecode/ops/tools/tick.mjs [--wp WP-xxx] [--force-timeout-scan] [--watcher-stale-min N] [--no-watcher-check]` |
| ops.board | Read `.litecode/ops/tools/board.md` |
| ops.emit_event | `node .litecode/ops/tools/emit_event.mjs --type --source --summary [--wp]` |
| ops.ack_event | `node .litecode/ops/tools/ack_event.mjs --id E-... --by <agent_id> [--wp]` |
| ops.kb_check | `node .litecode/ops/tools/kb_check.mjs [--node <key>] [--json] [--wp WP-xxx] [--stock-ref <rev>] [--selftest]` |
| ops.kb_nav | `node .litecode/ops/tools/kb_nav.mjs --key <key> [--json] [--wp WP-xxx]` · `--orphans` · `--selftest` |
| ops.kb_scaffold | `node .litecode/ops/tools/kb_scaffold.mjs --key <key> --domain <id> [--parent <key>] [--dir knowledge/...] [--write] [--json] [--wp WP-xxx]` |
| ops.wp_watch | `node .litecode/ops/tools/wp_watch.mjs start\|stop\|status\|once\|ensure\|test-callback\|watchdog-start\|watchdog-stop [--interval-sec N] [--heartbeat-sec N] [--force-timeout-scan] [--wp WP-xxx]` |
| ops.wp_watch_service | `node .litecode/ops/tools/wp_watch_service.mjs install\|refresh-env\|uninstall\|start\|stop\|boot\|status\|prove\|test-callback [--wp WP-xxx]` |
| ops.wp | `node .litecode/ops/tools/wp.mjs <new\|fill\|submit\|approve\|start\|deliver\|reject\|close\|set\|reassign\|tree\|summary\|history> --as <agent_id\|andy> ...`（v14，见「ops.wp 用法」；`--as` 自报无鉴权） |
| ops.wp.test | `node .litecode/ops/tools/wp.test.mjs [--json] [--keep]` |
| ops.issue_new | `node .litecode/ops/tools/issue_new.mjs --as <agent_id> --title T --domain D [--grade G] [--layer L] [--owner O] [--wp WP-x] [--json-file F] [--dry-run]` |
| ops.toolchain_check | `node .litecode/ops/tools/toolchain_check.mjs [--json] [--wp WP-x]` |
| ops.ops_harden.test | `node .litecode/ops/tools/ops_harden.test.mjs [--json] [--keep]` |

## ops.kb_check 用法（v7）

知识库校验。解析部分是 litecode 的 Node 复刻（`kb_parse.mjs` / `kb_scopes.mjs`，对应 `src/knowledge/{document,mentions,validate,fsref,index}.rs` 与 `src/engines/code_search/enclosing.rs`），以 `tests/fixtures/knowledge_refs/cases.json` 为准：`--selftest`（= `node .litecode/ops/tools/kb_check.test.mjs`）必须全过才算可用。不改 litecode 本体、不加依赖。

| 参数 | 作用 |
|---|---|
| （无） | 检查 `knowledge/` 全部节点，打印每条问题 + 末行 JSON 摘要 |
| `--node <key>` | 只报该节点的问题（键或 `knowledge/…md` 路径）；仍按全库解析（悬空/重复要看全库） |
| `--json` | 首行版本头之后输出一个 JSON：`{ok, nodes, errors, warnings, by_code, findings[]}` |
| `--wp WP-xxx` | 写 tool_traces |
| `--stock-ref <rev>` | 存量基线（默认取准入文档「基线提交」`e583ff2`）：该提交里已有的节点算存量，其余算新节点 |
| `--selftest` | 跑 cases.json 对照测试 |

退出码：0 = 无 error（可有 warning）；1 = 有 error；2 = 参数错误 / `--node` 找不到。

**error 与 warning**：error 必须修，`validate_all` 失败、知识类 WP 过不了 `wp_gate`；warning 只提示。新节点不满足准入规则 → error；存量节点同样问题 → 仅 warning。

| code | 级别 | 含义 |
|---|---|---|
| `ref_malformed` | error | 看起来像引用、但 litecode 会静默当普通文本（`[@key=`、`[ @`、全角括号/引号、未知属性、顺序不是 file→symbol→lines、key 不合规、file 为空、缺 `]`、`=` 两侧空格、key 与 file 混用…；`args.reason` 给出原因）。行内代码里的写法不算 |
| `ref_legacy` | error | 旧写法 `[[node : K]]`（KR-SYN-7） |
| `filename_mismatch` | error | 文件名主干 ≠ 键（litecode 为 warning；准入 §0「必须」，这里升为 error） |
| `dangling_relation` / `self_relation` | error | `[@ key]` 目标不存在 / 指向自己 |
| `missing_file` / `symlink_escape` / `unreadable_file` | error | 文件不存在（逐段精确匹配、区分大小写）/ 链接出工作区 / 不可读 |
| `missing_symbol` / `ambiguous_symbol` / `unsupported_language` | error | 符号链找不到 / 不唯一 / 语言不支持 |
| `invalid_lines` / `line_out_of_range` | error | 行号格式错（含符号引用带 lines）/ 越界 |
| `invalid_status` / `empty_key` / `duplicate_key` | error | 声明层问题 |
| `inactive_target` | warning | enabled 节点引用 pending/disabled 节点 |
| `symbol_drift` | warning | 符号正文与 HEAD 不同（准入 §6：视为证据失效，提交前重新核对） |
| `symbol_unverified` | warning | Go/C/C++/Java/C#/JSX/TSX 只做近似符号解析，找不到/不唯一时降为警告，需 litecode 索引确认 |
| `domain_missing` / `domain_unknown` | 新 error · 存量 warning | 声明块缺 `domain : <域 id>` / 域不在 board.json（team-ops 不产出节点） |
| `parent_missing` / `parent_multiple` | 新 error · 存量 warning | `上层：` / `父节点：` 二选一且恰好引用一个节点 |
| `baseline_missing` | 新 error · 存量 warning | 正文缺 `基线：<短哈希>` |
| `tag_unknown` | 新 error · 存量 warning | 「标签：[@ …]」不在五个里：上层 / 父节点 / 契约展开·框架展开·向下展开 / 相关 / 依据（模板 §7.1 的 `证据：` 视同 依据） |
| `new_not_pending` | error | 新节点必须 `status : pending`（除非该状态由 Andy 提交设定） |
| `new_status_by_andy` | warning | 新节点非 pending，但 `status :` 行最近一次改动是 Andy（itissika）提交且工作区未改动——状态归 Andy，只提示 |
| `dup_primary_suspect` | warning | 两个节点引用同一符号/行范围作证据，或 summary 完全相同 → 疑似主节点重复 |
| `ref_in_declaration` | warning | 声明块里的 `[@ …]` 不是引用 |
| `not_a_node` | warning | 知识根下的 `.md` 没有声明块，不是节点 |

接入：`validate_all` 每次都跑全库，error > 0 即失败；`wp_gate --to review|done` 对知识类 WP（domain=knowledge，或 WP 文本里写了 `knowledge/…md` / `[@ key="…"]` / `node:<key>`，或有节点 `domain :` 等于该 WP 的域，或传 `--kb-nodes`）要求涉及节点无 error，`--dry-run` 同样检查。基线结果：`.litecode/ops/artifacts/kb_check-baseline.md`。

## ops.wp_watch / wp_watch_service 用法（v11；v19 韧性；v21 开机自启）

### 中层怎么跑（WP-173）

```bash
# 在带有 LITECODE_WP_WEBHOOK_URL/KEY 的 shell 里（云电脑已注入；切勿 echo/打印）
cd /workspace/litecode
node .litecode/ops/tools/wp_watch_service.mjs install --wp WP-173
node .litecode/ops/tools/wp_watch_service.mjs status
node .litecode/ops/tools/wp_watch_service.mjs prove          # 实测：杀进程拉起 + 冷启动
node .litecode/ops/tools/wp_watch_service.mjs test-callback  # 向 GM 发一条测试事件
```

- **systemd 优先**：有 `systemctl` 时装 user unit `litecode-wp-watch.service`（`Restart=always`，`EnvironmentFile=~/.config/litecode/wp-watch.env`）。
- **无 systemd 的 box**（本云电脑为 tini/pod）：写同名 unit 文件备查，并启用 fallback svc（30s ensure 循环）+ `~/.profile` 登录钩子调用 `boot`。
- **密钥**：只写在 `~/.config/litecode/wp-watch.env`（0600，仓库外）。`refresh-env` 从当前 shell 重写；`uninstall --purge-env` 才删。绝不进 git。
- **提醒口径**：无 tick 定时提醒。只有 WP 停滞/升级（及 approval/acceptance、watcher_down/restarted）才发事件；空闲安静。
- **按 bot 分发**：`resolveWebhook(target)` 已留口，本次仍统一走 GM 回调；不做多 bot 注册实现。

### 进程命令（底层）

v19（2026-10-15 事故：快照回滚/VM fork 后 watcher 进程消失，无人调 tick，7 小时无超时告警）：
- `wp_watch.mjs ensure`：幂等。没跑/陈旧 pid → 起；进程在但心跳 >10 分钟（`--stale-min N`）→ 杀掉重起；健康 → 不动。
- `wp_watch.mjs test-callback`：向真实回调发一条 `team_ops.watcher_test`（target=gm）测试，只打印 http 码，写 wp_watch.log，不进 inbox。
- `wp_watch.mjs watchdog-start|watchdog-stop`：独立保活循环（每 `--watchdog-sec 60` 跑 ensure），pid 在 .tmp/wp_watchdog.pid；`status` 里有 watchdog 字段。
- 任何 `ops.wp` 写操作会顺带检查（watcher 死或心跳 >10 分钟 → detached ensure，5 分钟限频，记 `team_ops.watcher_restarted` source=ops.wp；`LITECODE_WP_NO_AUTOENSURE=1` 关闭）。
- v21：优先用 `wp_watch_service install`；机器重启后 fallback 靠登录钩子 / systemd；也可用 `wp_watch_service boot`。

常驻 watcher，把 WP 超时从「GM 主动扫」改成「回调触发」。纯 Node；服务层可选 systemd。

| 命令（在仓库根 `/workspace/litecode` 下） | 作用 |
|---|---|
| `node .litecode/ops/tools/wp_watch.mjs start` | 后台起 supervisor（detached），由它拉起 worker；已在跑则直接返回状态，不会双开（pidfile + `/proc` 存活校验） |
| `node .litecode/ops/tools/wp_watch.mjs status` | JSON：是否在跑、supervisor/worker pid、心跳时间与年龄、最近一轮、last_error、已送达/重试中数量 |
| `node .litecode/ops/tools/wp_watch.mjs stop` | SIGTERM supervisor → 它先停 worker 再删 pidfile |
| `node .litecode/ops/tools/wp_watch.mjs once [--force-timeout-scan]` | 前台只跑一轮（排障/联调） |
| `supervise` / `run` | 内部用：supervisor / worker 本体 |

参数：`--interval-sec`（默认 300）、`--heartbeat-sec`（默认 60）、`--force-timeout-scan`（忽略工作时段，仅联调）、`--wp`（写 tool_traces）。`start` 的参数会透传给 worker。

**每轮**：调 `scanAndEmitWpTimeouts`（与 tick 同一判定：HKT 工作日 9:45–17:45，in_progress/review 连续 >1h；事件 source=`ops.wp_watch`）→ 对 inbox 里**未 ack 且未送达**的 `team_ops.wp_timeout` 各 POST 一次。没有就什么都不发。

**环境变量**（只从进程环境读，绝不写进仓库 / git config）：`LITECODE_WP_WEBHOOK_URL`、`LITECODE_WP_WEBHOOK_KEY`。未配：每轮有待送事件时记一行 `delivery skipped`，继续运行，事件不标记已送达（配好后重启 watcher 即补发）。配置/改动 env 后需 `stop` 再 `start`（在带 env 的 shell 里）。

**回调**：`POST <URL>`，`content-type: application/json`，body：

```json
{"event_id":"E-…","wp_id":"WP-001","status":"review","age_minutes":73,"owner":"gm","reviewer":"andy","status_entered_at":"…Z","summary":"…","ts":"…Z"}
```

请求头：`Authorization: Bearer <KEY>`；`X-LiteCode-Event-Id: <event_id>`（幂等键）；`X-LiteCode-Timestamp: <unix 秒>`；`X-LiteCode-Signature: sha256=<hex(HMAC-SHA256(KEY, "<timestamp>.<raw body>"))>`。接收端校验任一即可，建议校验签名并拒绝时间戳偏差 >5 分钟的请求。超时 10 秒。

**送达语义**：只有 2xx 才写入状态文件 `delivered`；非 2xx / 网络错误记 `retry`，按 interval×2^(n-1) 退避（上限 1 小时）在后续轮次重试并记日志；事件被 ack 后不再重试。状态文件跨重启保留 → 同一事件永不重发。

**文件**（均在 `.litecode/ops/.tmp/`，目录内 `.gitignore` = `*`）：

| 文件 | 内容 |
|---|---|
| `wp_watch.heartbeat.json` | `{pid, supervisor_pid, ts, last_cycle_at, last_error, cycles, interval_sec, tools_version}`；每轮及每 60 秒刷新 |
| `wp_watch.log` | 追加日志，>1 MiB 轮转为 `wp_watch.log.1` |
| `wp_watch.state.json` | `{delivered:{event_id:{at,http_status,wp,attempts}}, retry:{event_id:{attempts,last_error,next_at}}}` |
| `wp_watch.pid` | supervisor pid（防双开） |

**稳定性**：worker 捕获 `unhandledRejection` / `uncaughtException` → 记日志、写 last_error、非零退出；supervisor 自动重启（1s 起指数退避，上限 60s，稳定运行 >60s 后复位）。服务层 `Restart=always`（systemd）或 fallback svc 30s ensure。**tick** 可查心跳作诊断，但 v21 起不是定时提醒主路径；机器重启后由 systemd / profile `boot` 拉起。


## ops.kb_nav 用法（v12）

按 key 摸完整引用链（出边 / 入边），并提示缺边、无父节点、孤儿。复用 `kb_parse.mjs` 的 `computeOnce` / `extractCites`；父节点标签与 `kb_check` 准入一致（`上层` / `父节点`）。不改 litecode 本体。

| 参数 | 作用 |
|---|---|
| `--key <key>` | 查该节点的 outbound（`[@ key]`）、inbound（谁引用它）、缺边（出边目标不存在）、父节点/基线/孤儿提示；顺带汇总 file/symbol 引用数量 |
| `--orphans` | 列出全库无人引用的节点（入边为 0；自引用不算入边） |
| `--json` | 版本头后输出结构化 JSON |
| `--wp WP-xxx` | 写 tool_traces |
| `--selftest` | 跑 `kb_nav.test.mjs`（临时夹具 + live smoke + scaffold） |

退出码：0 成功；1 key 不存在 / 无知识根；2 参数错误。

**提示字段**：`missing_targets`（出边悬空）、`parent.missing` / `parent.multiple`（无或多余上层/父节点；存量且无父节点带 `stock_root_hint`）、`orphan`（无入边）、`baseline`。

示例：

```
node .litecode/ops/tools/kb_nav.mjs --key session-model
node .litecode/ops/tools/kb_nav.mjs --orphans --json
node .litecode/ops/tools/kb_nav.mjs --selftest
```

## ops.kb_scaffold 用法（v12）

新知识节点脚手架。生成声明块（`node` / `status : pending` / `domain`）+ `上层：`（或 `--tag 父节点`）引用 + `基线：<短哈希>` + 关系标签注释。默认 dry-run 打印到 stdout；`--write` 仅在目标文件不存在时创建（已存在则拒绝，不覆盖）。

| 参数 | 作用 |
|---|---|
| `--key <key>` | 节点键（须符合知识键规则） |
| `--domain <id>` | board.json **产品域**（禁止 `team-ops`） |
| `--parent <key>` | 父节点键；省略则留下 `<parent-key>` 占位 |
| `--dir knowledge/...` | 目录或完整 `.md` 路径；默认 `knowledge/<key>.md` |
| `--tag 上层\|父节点` | 父关系标签，默认 `上层` |
| `--write` | 落盘（文件已存在 → 失败） |
| `--json` / `--wp` / `--selftest` | 同其他工具；`--selftest` 走 `kb_nav.test.mjs --scaffold-only` |

示例：

```
node .litecode/ops/tools/kb_scaffold.mjs --key my-node --domain harness-core --parent session --dir knowledge/contracts
node .litecode/ops/tools/kb_scaffold.mjs --key my-node --domain harness-core --parent session --dir knowledge/contracts --write
```

## ops.wp 用法（v14，ops.wp v2：方案审批 ≠ 完工验收）

新模型包 = 声明了 `parent` 或 `goal`（或带 `history`）的包；老 WP（WP-001..019）不受影响。

```
drafting --submit--> awaiting_approval --approve--> ready --start--> in_progress --deliver--> review --close--> done|blocked
                 <--reject(方案退回)--                              <--reject(验收退回，可 --reopen 子包)--
```

```
node .litecode/ops/tools/wp.mjs new     --as gm|andy --title T --goal G --owner O --approver A                  # 顶层包
node .litecode/ops/tools/wp.mjs new     --as <父包owner> --parent WP-x --title T --goal G --owner O [--approver A] # 子包，approver 默认父包 owner
node .litecode/ops/tools/wp.mjs fill    --as <owner> --id WP-x --text T|--content-file F [--kind direct|split]   # 仅 drafting
node .litecode/ops/tools/wp.mjs submit  --as <owner> --id WP-x [--note N]      # 交方案 → awaiting_approval；split 至少 1 个子包（子包可仍 drafting）；父包方案须已批
node .litecode/ops/tools/wp.mjs approve --as <approver> --id WP-x [--note N]   # 批方案 → ready：可执行、可下派子包
node .litecode/ops/tools/wp.mjs reject  --as <approver> --id WP-x --note N     # 方案退回 → drafting
node .litecode/ops/tools/wp.mjs start   --as <owner> --id WP-x                 # ready → in_progress（本包及父包方案已批）
node .litecode/ops/tools/wp.mjs deliver --as <owner> --id WP-x [--note N]      # 送验收 → review；子包须全部 done/blocked（cancelled 也算封）
node .litecode/ops/tools/wp.mjs close   --as <approver> --id WP-x --to done|blocked [--note N]   # 验收通过封包（done 只能从 review）；有 blocked 须 note
node .litecode/ops/tools/wp.mjs reject  --as <approver> --id WP-x --note N [--reopen WP-a,WP-b]  # 验收退回 → in_progress（方案仍有效），子包重开回 drafting
node .litecode/ops/tools/wp.mjs close   --as <approver> --id WP-x --to cancelled [--note N]      # 任意时刻取消
node .litecode/ops/tools/wp.mjs tree|summary|history [--id WP-x] [--json]
node .litecode/ops/tools/wp.mjs set     --as <owner|approver> --id WP-x --field artifact_path|handoff_path|progress|result|done_when|domain --value V   # v3；v4 domain 须为 board.domains 已有 id；
node .litecode/ops/tools/wp.mjs reassign --as <approver|父包 owner> --id WP-x --owner O [--note N]   # v4；O 须为已注册 agent；只改 owner；已封包不可改；v5 已封包仅 domain 可重新归类
#路径须为 .litecode/ops/...；done_when 用 ; 分隔；已封包不可改
```

- 父包方案未批之前：子包可以 new/fill，但不能 submit 方案，也不能 start（wp start 与 wp_gate in_progress 都拦）。
- 每次写入：board.json → validate.mjs（失败回滚）→ history（append-only：create/fill/submit/approve/start/deliver/reject/return/reopen/close）+ log.jsonl `wp_lifecycle` → 刷新「WP 状态」树 → tool_traces。submit 发 `team_ops.approval_requested`，deliver 发 `team_ops.acceptance_requested`。
- wp_gate 对新模型包：进 in_progress 须本包方案已批且父包方案已批；推 review/done 须子包全封；done 只能 approver（`--source`）推。
- 超时：两种等待（awaiting_approval 方案待批、review 待验收）以及 in_progress，工作时段 >1h → `team_ops.wp_timeout` 发给 approver；未 ack 超 5 分钟再发一条 `escalated:true` 给父包 approver，**顶层只升到 gm 为止**（升级链里没有 andy 级；要不要告诉 Andy 由 GM 判断）。老 WP 不升级。webhook 字段：`level` `parent` `path` `target` `escalated`。
- **已知边界：身份靠 `--as` 自报，不做真实鉴权，靠自律；审计看 history/log 的 actor。**
- **工具需求指挥链**：域 bot 的工具需求报给本域中层；中层批过的直接转给工具集管家；中层自己的需求报 GM；GM 自己的需求先问 Andy；工具集管家只接上级转来的需求。
- 测试：`node .litecode/ops/tools/wp.test.mjs`（临时副本，跑完清理；`--keep` 保留）。

## v15 告警 / 事件 / issue / 工具链（WP-032）

**超时告警**
- 卡点定义：awaiting_approval、review 由 approver 动手；in_progress 由 owner 动手。有未封子包的 in_progress 包**不算卡点**，它在等子包。
- 每条父子链只发一条，发给离卡点最近、真正要动手的人（最深的 stale 节点）。被合并掉的祖先包记在 `meta.merged`。
- matter = `wp|status|status_entered_at|target`：
  - 未 ack 的不重发（新模型包未 ack 超过 5 分钟，会升级一次，顶层止于 gm）。
  - 已 ack 的 **60 分钟内不重发**，老 WP 同样适用。
  - 状态变化算新 matter。
- **所有告警都走 webhook**（GM 例程转私信）。payload 带 `target`（谁该动手）、`type`、`actor`、`merged`、`matter_key`，v11/v13 的字段不变。target 永远不是 andy，approver=andy 时按 gm 计。

**事件写入与投递**
- 只有一个发事件的进程：wp_watch。tick 在 watcher 健康时只报告（`report_only`）。watcher 心跳失效时，tick 自动执行 `wp_watch start` 并记一条 `team_ops.watcher_restarted`（`--no-autostart` 可关闭）；起不来再兜底 emit。
- 事件写入：`events/.inbox.lock` 加锁，写 tmp，fsync，rename，再 fsync 目录。可投递类型（超时、approval/acceptance_requested、watcher_down/restarted）同时追加到 `events/outbox.jsonl`（fsync）。**落盘之后才可能被 POST。**
- wp_watch 启动时和每个周期都会排空 outbox 里「没送达、没 ack」的条目。delivered/retry 状态以 fsync 方式写入。按 `x-litecode-event-id` 幂等。

**编译环境与共享 target 锁（v18）**
- 一律 `source .litecode/ops/tools/cargo_env.sh` 或直接用 `.litecode/ops/tools/cargo_locked.sh [-t SECS] <cargo args>`。
- HOME 必须是绝对路径：`/workspace/litecode/.tmp/home`（cargo 1.99 遇到相对 HOME 会 panic）。
- 先 unset 旧的 RUSTUP_HOME/CARGO_HOME/RUSTUP_TOOLCHAIN，再设为 `/workspace/litecode/.litecode/ops/.toolchain/rustup` 和 `.../cargo`（绝对路径）。不要自己装工具链。
- CARGO_TARGET_DIR：默认共享主 target `/workspace/litecode/target`；独立实验（release/bench）用 `.tmp/<wp>/target-*`。
- 锁：**只用 `/workspace/litecode/.tmp/cargo-target.lock`**（flock，包住整个 cargo 调用；用独立 target 也要拿，作内存护栏）。`.tmp/target.lock` 没有任何脚本/文档引用，是冗余空文件，不要用。重型任务仍需 backend_mid 排队许可。
- 磁盘回滚或编译报 crate 文件缺失（如 E0583）时：`.litecode/ops/tools/registry_verify.sh`（只读）→ `--fix` 从 cache 重解压。不编译。`--quick` 只做 stat 比对（按 tarball 文件清单 manifest，健康约 0.2s）。
- v20：`cargo_locked.sh` 取锁前自动跑 `--quick`，有损坏才 `--fix` 并只打印一行；`LC_SKIP_VERIFY=1` 跳过。
- **反复丢文件的原因（2026-10-15 取证）**：快照/回滚后，registry/src 里**所有名为 `target` 或 `build` 的目录**整体消失（cc 的 src/target、thiserror/rustversion 的 build/、cxx 的 book/src/build、ort-sys 的 build/download、zstd-sys 的 zstd/build 共 12 个 crate），其它文件都在，cache/*.crate 完好；全 registry 里已找不到任何同名目录。仓库 .gitignore 第 2 行 `target` 会匹配任意层级，`build` 不在其中，所以更像快照/恢复工具按「常见构建产物目录名」过滤。结论是推断（没法看到恢复工具本身）。对策：verify 按 tarball 文件清单比对（不是只看目录在不在），cargo_locked 自动修。
- **g++（v20）**：两条路径互不冲突。① 系统 g++：`sudo apt-get install g++`（backend_mid 本次开机装的 14.2，重启即丢）。② 用户态兜底：`.toolchain/debs`（缓存的 .deb）→ `.toolchain/gxx-root`（dpkg -x）→ `.toolchain/gxx-bin/{g++,c++}`（wrapper，自带 -B/-isystem/-L）。`cargo_env.sh` 在**没有系统 g++ 或 LC_FORCE_USERSPACE_GXX=1** 时自动把 gxx-bin 放进 PATH 并设 CXX；有系统 g++ 时静默不动。`registry_verify.sh --fix` 会离线从 debs 重建 gxx-root/gxx-bin。
- **重启/快照恢复后清单**：`.litecode/ops/tools/registry_verify.sh --fix`；`node .litecode/ops/tools/wp_watch.mjs ensure && node .litecode/ops/tools/wp_watch.mjs watchdog-start`（需 shell 有 webhook env，v19）。

**Issue 编号**
- 新建 issue 一律用 `ops.issue_new`：加锁，取 max(文件, git HEAD, `issues/.reserved.json`)+1，独占创建，绝不覆盖。禁止手写编号。
- validate_all 会查文件名和 id 是否一致、id 是否重复、是否用了保留号。

**工具链**
- `~/.cargo/bin/*` 已换成 wrapper：先设 RUSTUP_HOME/CARGO_HOME 默认值，再 exec ops toolchain。`~/.rustup` 是指向 `.litecode/ops/.toolchain/rustup` 的软链。原文件备份在 `~/.litecode-toolchain-backup-20261012-184910/`。
- cargo 报错时先跑 `toolchain_check`。

## 待发掘

- kb_check：Go / C / C++ / Java / C# / JSX / TSX 的符号解析是近似实现（仓库目前没有此类符号引用；Rust/TS/JS/Python 已与 litecode 的 tree-sitter 结果逐文件核对一致）
- kb_check：漂移（symbol_drift）用 git CLI 读 HEAD，litecode 用 git2；快照仓库判定（git_dir_is_snapshot）未复刻
- wp_watch：按 bot 分发回调尚未实现（WP-173 只留 resolveWebhook 口）；多 webhook 注册另立包
- wp_watch 与 tick 都会写 events/inbox.json，未加文件锁（写入很短，冲突概率低）
- 使用中发现的其它缺项记在这里，由工具集管家热更

## 边界

- 仅 team-ops 维护；产品 bot / 群对本启动无感知，直至 Andy 群内统一启动
## tools_version 20（WP-171，2026-10-15）
- 新增 `ops.init_env`：`bash .litecode/ops/tools/init_env.sh`，新电脑一键初始化，流程与证据见 `.litecode/ops/artifacts/WP-171/init-flow.md`。
- `cargo_env.sh` 的 LC_ROOT 可覆盖（默认取脚本所在仓库）；仓库加 `rust-toolchain.toml` pin。
- `toolchain_check` v2：env -i 行先 source cargo_env.sh，必须解析到 ops .toolchain 的 cargo，且版本等于 pin。
- 需求官撤销：`demand/` 已删除，tick 和 validate_all 不再读取；需求改由各 bot 的直属领导审批。
- 全局内存规则（云电脑兜底，2026-10-15）：`LC_MIN_MEM_GIB` 默认 **2**（启动门槛/余量），`LC_KILL_MEM_GIB` 默认 **1**（看门狗杀线）。Andy「留 2GiB」指开跑前余量，不是杀线；run5 曾把两条都设成 2 误杀正常编译峰值（谷底 1.96GiB）。仍可用环境变量覆盖。
- init_env v2：测试阶段改为 4 步全部执行（cargo test 加 --no-fail-fast），任一失败则 exit 6，并生成 `test_failures.txt`（由 ops.test_failures 产出，每条为 测试名 @ 文件:行 — 消息）；前面各步仍 fail-fast。
- **WP-172（进行中）重型默认改走 GitHub Actions**：`.github/workflows/ci-linux.yml`（cargo + web + desktop）与 `ci-init-env.yml`（干净 init_env）均为 **仅 `workflow_dispatch`**，禁止 push/PR 自动全量；现有 tag→release 自动保持。中层用法见 `.litecode/ops/artifacts/WP-172/gh-workflow-run.md`。云电脑门禁保留作兜底，不删。

## tools_version 21（WP-173，2026-10-15）
- 新增 `ops.wp_watch_service`：把 wp_watch 装成开机自启常驻服务（systemd user 或 fallback）。
- webhook 环境文件：`~/.config/litecode/wp-watch.env`（0600，仓库外）；unit 模板在 `tools/systemd/litecode-wp-watch.service`。
- 提醒：去掉 tick 定时提醒依赖；仅停滞/升级等事件由常驻 watcher 发出。
- 按 bot 分发回调：设计口 `resolveWebhook`，本次不做实现。

## tools_version 22（WP-174，2026-10-15）
- `eventStillDeliverable`：approval 仅 awaiting_approval；acceptance 仅 review；timeout 仅 open clock；closed/缺失跳过。
- 冷启动 cycle 对 obsolete outbox 记 `delivered.skipped`，不 POST，避免冲垮 GM webhook。
