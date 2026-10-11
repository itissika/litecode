# 知识库引用规范（Knowledge References）

状态：规范 v2（2026-10-11，Andy 已拍板；同日修订 KR-SYM-6：符号引用不带 `lines`）。契约表：`tests/fixtures/knowledge_refs/cases.json`（v2）；后端锁定测试：`src/knowledge/spec_tests.rs`；常驻索引：`src/knowledge/index.rs`。

本文是知识库「引用解析」的唯一权威。每条规则有编号，契约表里每个 case 都要指向一条规则，测试会检查编号确实在本文中定义过、状态表与代码可产出的状态完全一致。

## 0. 不变量

- **KR-INV-1** 权威状态是**磁盘的物理状态**。一个引用是否成立，只看工作区目录在磁盘上的真实内容，不看 Explorer 是否显示、不看 `.gitignore` / `files.exclude`、不看编辑器缓冲区、不看 git 索引。
- **KR-INV-2** 后端状态是**唯一真相**。同一份磁盘状态（同一 HEAD）在任何平台、任何调用方得到同一结论：turn 提醒、`knowledge` 工具、`/symbol-refs`、`GET /api/workspace/knowledge`、用户消息展开都读同一个常驻索引（或同一套规则的一次性计算），不允许入口不同结论不同。
- **KR-INV-3** 前端**消费**后端快照，不自行判定引用状态（见第 10、11 节）。前端可以即时渲染正文，但引用状态只来自快照；快照未就绪时显示 `checking`。
- **KR-INV-4** 规则靠测试锁住。新增或修改规则时，同步改 `cases.json` 和本文；后端不符合的地方在第 7 节列为分歧并在表中标 `divergence`（当前没有）。

## 1. 引用的语法

引用只出现在节点正文（`node` 声明块之后的 markdown）里，也出现在用户消息里（同一语法，见 KR-SYN-9）。

- **KR-SYN-1** 节点引用：`[@ key="K"]`。`K` 去首尾空白后必须满足键规则：`^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$`（字母/数字/`_`/`-`，词之间单个空格）。不满足时整段是普通文本，不是引用，也不报错。
- **KR-SYN-2** 写法是固定的：`[@` 后一个空格；属性名只认 `key`、`file`、`symbol`、`lines`；顺序固定为 `file` → `symbol` → `lines`；出现任何未知属性（如 `label`、`id`）或顺序不对，整段是普通文本。
- **KR-SYN-3** 文件引用：`[@ file="P"]`。`P` 去首尾空白；为空则是普通文本。
- **KR-SYN-4** 符号引用：`[@ file="P" symbol="C"]`。`C` 去首尾空白；为空时等同文件引用。`C` 是作用域链，段之间用 ` › `（U+203A，两侧各一个空格），如 `impl Store › fn save`。符号引用只靠链定位，**不带 `lines`**（KR-SYM-6）。
- **KR-SYN-5** 行号语法只有两种：`lines="A"`、`lines="A-B"`。A、B 是十进制正整数：只含 ASCII 数字 `0-9`，不以 `0` 开头（因此没有 0），不带符号（没有 `+`/`-` 前缀），无空格，无开区间（`1-`、`-3`），不超过 u32（4294967295），且 `A ≤ B`。其余一切写法都是**格式错误**：引用照常抽取（带 `symbol` 的是符号引用，否则带 `lines` 的是范围引用），原文记为 `bad_lines`，解析结果为 `invalid_lines`（KR-RANGE-2）。只有 `file` + `lines`、没有 `symbol` 的是**范围引用**；行号只出现在范围引用里。符号引用上写了 `lines`（无论格式对不对）同样是格式错误（KR-SYM-6）。`A-A` 规范化为 `A`。
- **KR-SYN-6** 代码里不是引用：围栏代码块（行首最多 3 个空格，然后 ≥3 个 `` ` `` 或 `~`，用同字符、不短于开栏的围栏关闭；未关闭则直到文末）和行内代码（成对的等长反引号串）里的内容一律不是引用。未闭合的单个反引号不构成行内代码。缩进 4 个空格的反引号行不是围栏。
- **KR-SYN-7** 旧写法 `[[node : K]]` 和声明块里的 `ref :` 行不是引用。迁移旧写法必须是一次显式写入，不能作为「读取」的副作用（见 F4）。
- **KR-SYN-8** 抽取顺序：按出现顺序逐行抽取，同一行内按起始位置排序；顺序抽取保留重复，解析时按 `ref_key` 去重（KR-VAL-2）。
- **KR-SYN-9** 用户消息里的引用与正文同语法、同解析规则；区别只在用途（展开为上下文快照），不在解析。

### 1.1 引用身份 `ref_key`

- **KR-KEY-1** 每个引用有一个确定的身份 `ref_key`，前后端、契约表、API 一律用它对齐：

  | kind | ref_key |
  |---|---|
  | `node` | `node:<去空白的 key>` |
  | `file` | `file:<规范化路径>` |
  | `symbol` | `sym:<规范化路径>\0<去空白的链>` |
  | `range` | `range:<规范化路径>\0<lines>` |

  规范化路径 = 去首尾空白、`\` 换成 `/`，其余原样（不折叠 `//`、不解析 `..`，非法路径照样有 key）。`\0` 是 U+0000。`<lines>` 合法时取规范化形式（`5-5` → `5`），格式错误时取原文。符号引用的 key **永远不含** `lines`：`[@ file="P" symbol="C" lines="…"]` 的 key 仍是 `sym:P\0C`（它本身是 `invalid_lines`，见 KR-SYM-6）。原文完整保留在 `raw`（引用在正文中的原始片段，如 `[@ file="src\a.rs"]`）。

## 2. 路径

- **KR-PATH-1** 合法路径必须是工作区相对路径：非空；不超过 512 个 Unicode 标量；不以 `/` 开头；第二个字符不是 `:`（盘符）；按 `/` 切分后每段非空且不是 `.` 或 `..`（因此 `src/`、`src//a`、`./a`、`a/../b` 都不合法）。不合法路径直接得到 `missing_file`，不访问磁盘。
- **KR-PATH-2** `\` 视为 `/`。去重按 `ref_key`（即规范化路径）进行。
- **KR-PATH-3** 存在性 = 磁盘上该路径存在（文件或目录都算）。被 gitignore、`files.exclude` 隐藏的文件、以 `.` 开头的文件和目录都照常存在。
- **KR-PATH-4** 名称按磁盘上目录项的**原样名称精确匹配**：逐段读目录列表比较，区分大小写，**所有平台一致**；不做 Win32 尾部 `.`/空格裁剪，不认 `::$DATA` 之类的流名，不做 Unicode 规范化。`SRC/a.rs` 不能命中 `src/a.rs`，`src/a.rs.` 不能命中 `src/a.rs`。
- **KR-PATH-5** 不跟随改名或移动：路径不存在就是 `missing_file`，不查 git 历史去猜新位置。
- **KR-LINK-1** 符号链接（含 Windows 符号链接 / junction）被跟随：最终目标存在且在工作区内（规范化后以工作区规范路径为前缀）时按目标判断；目标在工作区外为 `symlink_escape`（错误，适用于所有路径型引用，包括纯文件引用）；断链为 `missing_file`。

## 3. 节点引用与声明

- **KR-NODE-1** 目标 = 语料中去空白后声明键**完全相等**（区分大小写）的节点；找不到为 `dangling_relation`（错误）。
- **KR-NODE-2** 引用自己为 `self_relation`（错误）。
- **KR-NODE-3** 只有 `enabled` 的节点引用 `disabled` 或 `pending` 的目标时报 `inactive_target`（警告，不计入徽标和 turn 提醒的类别）。非 enabled 的节点引用谁都不报这一条。
- **KR-NODE-4** 键重复时，重复的每个节点各报 `duplicate_key`（错误）；引用该键的节点不另报，解析到按路径排序的第一个，结果确定。
- **KR-NODE-5** 语料成员：知识根（`knowledge/` 存在则用它，否则 `.litecode/knowledge`）下递归所有扩展名为 `.md`（不区分大小写）的文件，按磁盘真实目录遍历，跳过名字以 `.` 开头的文件和目录；不受 gitignore、`files.exclude` 影响。
- **KR-DECL-1** 声明层问题（`ref_key` 为 `null`，契约表里用 `ref`）：`invalid_status`（错误，status 不是 enabled/disabled/pending，`args.status`）；`empty_key`（错误，键不满足 KR-SYN-1 的键规则，`args.key`）；`duplicate_key`（错误，`args.key`）；`filename_mismatch`（警告，文件名主干 ≠ 键，`args.key`、`args.filename`）。没有声明块或键为空的 `.md` 不是节点，而是「未识别文件」（KR-LAYOUT-1）。
- **KR-LAYOUT-1** 节点身份是**相对知识根的规范化路径**（`/` 分隔），键是另一个字段。文件夹树 = 知识根下所有（非 `.` 开头的）目录，加上每个节点和未识别文件所在目录的祖先链；每个文件夹 `{id: 相对路径, name: 末段, parent_id}`。未识别文件 = 不是节点的 `.md` 文件（`{path, name, folder_id}`）；非 `.md` 文件不出现。

## 4. 文件、符号、范围引用的解析

一个引用只有一个状态：按下列顺序取**第一个**失败：路径（KR-PATH / KR-LINK）→ 行号格式（KR-SYN-5；符号引用带 `lines` 也在这一步，KR-SYM-6）→ 可读性（KR-READ-1，纯文件引用不查）→ 语言（KR-SYM-5）→ 符号（KR-SYM-2/3/4）→ 越界（KR-RANGE-2）→ 漂移（第 5 节）。

- **KR-READ-1** 唯一读策略：符号、范围引用需要读文本。可读 = 是普通文件、大小 ≤ 10 MiB（10 485 760 字节，正好 10 MiB 可读）、内容是合法 UTF-8（开头 BOM 忽略）。否则为 `unreadable_file`（错误），`args.reason` 为 `directory` / `larger than 10 MiB` / `not UTF-8 text`，消息为 `"<path>" is not a readable text file (<reason>).`。目录、超大文件、二进制文件对**纯文件引用**仍然算存在（`ok`）。用户消息展开、`/symbol-refs`、索引共用此策略（`knowledge::fsref`），文字与状态一致。
- **KR-SYM-1** 符号引用先按第 2 节判断文件；文件不合法或不存在时只报 `missing_file`（或 `symlink_escape`），不再报符号。
- **KR-SYM-2** 用 tree-sitter 列出文件中所有命名作用域，链**完全相等**（区分大小写）的唯一命中为成立；零命中为 `missing_symbol`（错误）。
- **KR-SYM-3** 多于一个命中为 `ambiguous_symbol`（错误）。这是独立状态码，不再同时报 `missing_symbol`。
- **KR-SYM-4** 旧链兼容：`impl T › …` 在没有精确命中时，若恰好一个 `impl Trait for T › …` 匹配则成立，多个则 `ambiguous_symbol`。
- **KR-SYM-5** 不支持的语言（tree-sitter 没有对应语法，按扩展名判断）为 `unsupported_language`（错误），消息 `该语言不支持符号解析 ("<path>").`。支持：rs、py、go、js/jsx/mjs/cjs、ts、tsx、c/h、cc/cpp/cxx/hpp/hh/hxx、java、cs。
- **KR-SYM-6** 符号引用**不带** `lines`：符号只按链定位，行号只属于范围引用，两者不混用。`[@ file="P" symbol="C" lines="…"]` 是格式错误，状态 `invalid_lines`（错误），`args.reason` = `lines not allowed on symbol reference`，`args.lines` 为原文，消息 `Symbol citations do not take lines (lines="<raw>"). Remove lines, or cite a line range as [@ file="<path>" lines="A-B"].`。判定位置与 KR-SYN-5 相同（路径之后、可读性之前），不论 `lines` 本身格式是否合法；该引用不做符号解析、不报漂移。`ref_key` 仍为 `sym:<路径>\0<链>`，API 里该引用的 `lines` 字段为 `null`。写入方（`knowledge` 工具、提示词、前端插入）一律不得在符号引用上写 `lines`。
  - 状态码选择：复用 `invalid_lines`，不新增状态码。理由：这是 `lines` 属性的**写法错误**（与 KR-SYN-5 同类：不看磁盘内容就能判定），前端按 `invalid_lines` 已有的样式展示即可；两种情况靠 `args.reason` 区分：范围引用格式错误为 `invalid line span`，符号引用带 `lines` 为 `lines not allowed on symbol reference`。
- **KR-RANGE-1** 范围引用先按第 2 节判断文件，再按 KR-READ-1 判断可读。范围不追踪漂移。
- **KR-RANGE-2** 范围引用的 `lines` 格式错误为 `invalid_lines`（错误，消息 `Lines "<raw>" are not valid. Write lines="A" or lines="A-B" (decimal, 1 <= A <= B).`）；`B` 大于文件行数为 `line_out_of_range`（错误，`args.line_count`）。行数 = 按 `\n` 切分的行数（末尾换行不另算一行，空文件 0 行，CRLF 算一行）。用户消息展开时越界同样报错，不再截断。

## 5. 漂移（HEAD 对照）

- **KR-DRIFT-1** 符号存在时，取 HEAD 中同一路径文件里同一链的唯一作用域正文，与磁盘当前正文比较；不同则 `symbol_drift`（警告）。HEAD 由 git2（libgit2）读取，按 blob oid 缓存，不启动 git 子进程。
- **KR-DRIFT-2** 比较的是作用域正文，逐行去掉行尾空白，按行比较（因此 CRLF 与 LF、行尾空格、作用域外的改动、整体上下移动都不算漂移）。
- **KR-DRIFT-3** HEAD 中没有该文件，或 HEAD 中该链不存在 / 不唯一，都算 `symbol_drift`。
- **KR-DRIFT-4** 文件在磁盘上已删除时按 KR-SYM-1 只报 `missing_file`。
- **KR-DRIFT-5** 没有 git 仓库、HEAD 未出生（无提交）、仓库是快照仓库时漂移未知，按成立处理。漂移基线永远是 HEAD，与知识节点本身是否提交无关。

## 6. 状态分类

| 状态 | 级别 | 适用 | 规则 |
|---|---|---|---|
| `ok` | — | 全部 | 成立 |
| `checking` | — | 全部 | 仅出现在 `ready:false` 的预览快照里：事实尚未计算完 |
| `dangling_relation` | error | 节点 | KR-NODE-1 |
| `self_relation` | error | 节点 | KR-NODE-2 |
| `inactive_target` | warning（不计徽标） | 节点 | KR-NODE-3 |
| `missing_file` | error | 文件/符号/范围 | KR-PATH-1/3/4/5、KR-LINK-1（断链） |
| `symlink_escape` | error | 文件/符号/范围 | KR-LINK-1 |
| `invalid_lines` | error | 符号/范围 | KR-SYN-5、KR-RANGE-2（范围：`args.reason` = `invalid line span`）；KR-SYM-6（符号带 `lines`：`args.reason` = `lines not allowed on symbol reference`） |
| `unreadable_file` | error | 符号/范围 | KR-READ-1 |
| `unsupported_language` | error | 符号 | KR-SYM-5 |
| `missing_symbol` | error | 符号 | KR-SYM-2 |
| `ambiguous_symbol` | error | 符号 | KR-SYM-3/4 |
| `symbol_drift` | warning | 符号 | KR-DRIFT-1/3 |
| `line_out_of_range` | error | 范围 | KR-RANGE-2 |
| `invalid_status` | error | 声明 | KR-DECL-1 |
| `empty_key` | error | 声明 | KR-DECL-1 |
| `duplicate_key` | error | 声明 | KR-NODE-4 |
| `filename_mismatch` | warning | 声明 | KR-DECL-1 |

新增状态（v2）：`symlink_escape`、`invalid_lines`、`unreadable_file`、`unsupported_language`、`ambiguous_symbol`（独立码）、`line_out_of_range`。

- **KR-VAL-1** 节点的 `severity`：任一 error → `error`；否则任一非 `inactive_target` 的 warning → `warning`；否则 `null`。turn 提醒类别按 错误 > 警告 > pending > ok。
- **KR-VAL-2** 去重：同一节点内按 `ref_key` 去重，每个 `ref_key` 一条结果；重复时取最差（error > warning > ok）的那一次。

## 7. 分歧

处理原则：一律以磁盘真实状态为准，后端是唯一真相。

后端分歧：**无**。v1 列出的 B1–B5 已全部修复，契约表中不再有 `divergence` 标记（`listed_divergences_still_diverge` 保证以后标了就必须真的不同）：

- **B1** 已修：存在性逐段按目录项原名精确比较（`knowledge::fsref::lookup`），各平台区分大小写。
- **B2** 已修：同 B1，尾部 `.` 不再被 Win32 规范化命中。
- **B3** 已修：`parse_line_span` 只接受 KR-SYN-5 的形式。
- **B4** 已修：只剩一套读策略（KR-READ-1，`knowledge::fsref`），10 MiB 上限对索引、展开、`/symbol-refs` 一致；由 `oversized_and_binary_files_are_unreadable_everywhere` 锁定。
- **B5** 已修：目录上的范围 / 符号引用在索引、展开文字、`/symbol-refs` 中都是 `unreadable_file (directory)`；由 `directory_texts_match_the_status` 锁定。

前端分歧（契约表中以 `frontend` 注明）。**全部通过「前端改为消费后端快照」消除**（第 10、11 节），前端不再自行判定：

- **F1** 文件存在性经 Explorer 列表（`fetchTree`）判断，被隐藏的文件被判为不存在。→ 改读快照 `refs[].status`。
- **F2** 名称比较大小写不敏感。→ 同上。
- **F3** 语料成员经 Explorer 列表遍历。→ 节点、文件夹、未识别文件改读快照 `nodes` / `folders` / `unknown`。
- **F4** 打开面板时 `upgradeKnowledgeMarkdown` 改写旧写法并写回磁盘（读取的副作用）。→ 删除写回；迁移只能由用户显式触发。
- **F5** 存在性未检查完时的过渡态、请求失败时的错误判定。→ 快照 `ready:false` 时显示 `checking`；请求失败保留上一份快照，不自行判定。
- **F6** `parseLineSpan` 用 `Number()`，接受超过 u32 的行号。→ 状态改读快照；本地抽取（仅用于即时渲染）按 KR-SYN-5 实现。
- **F7** 符号检查走 `/symbol-refs`。→ 改读快照（`/symbol-refs` 保留，但已与快照同引擎）。

## 8. 已拍板（2026-10-11）

1. KR-PATH-4：精确名称、区分大小写、所有平台一致；尾部 `.` 等 Windows 规范化不命中。
2. KR-SYN-5 / KR-RANGE-2：行号语法固定，格式错误与越界都是错误。
3. KR-SYM-5：不支持的语言是错误，消息「该语言不支持符号解析」。
4. KR-LINK-1：跟随链接，目标须在工作区内，越界是错误。
5. KR-READ-1：一套读策略，文字与状态一致。
6. 后端是唯一真相，前端消费后端快照。
7. KR-SYM-6：符号引用不带 `lines`，符号 / 行号两种定位不互相串；带了就是格式错误（`invalid_lines`）。取代 v1「符号引用上的 `lines` 只校验格式、作显示快照」的规则。

仍开放：无。

## 9. 常驻索引（KnowledgeIndex）

- 服务启动（`serve::router::listen`）时后台线程建索引，不阻塞首轮；未就绪时首轮跳过知识状态提醒。
- 增量：订阅工作区变更总线（`subscribe_changes`，watcher 300 ms 防抖）。只有落在知识根、被引用路径（含其祖先目录）、`.git/HEAD|refs/heads|packed-refs` 上的变更会入队；总线丢消息（lagged）按全量处理。另有每 2 s 一次的后台全量 stat 兜底（覆盖 watcher 排除的路径、工作区之上的仓库）。
- 缓存：知识 `.md` 按 (len, mtime) 缓存解析结果；被引用文件按 (路径, len, mtime) 缓存文本和作用域表（每份内容最多解析一次）；HEAD 按 commit 缓存树，blob 按 oid 缓存文本和作用域表；符号结论按（文件代数, HEAD blob）缓存。stamp 相同而内容不同（同一时间戳粒度内的同长度改写）检测不到，属已知局限。
- 每轮（`append_knowledge_status_for`、`exec.rs` 认领排队消息的路径）只读快照：最多等 50 ms 让已入队的变更发布，超时用上一份快照；零 git 子进程、零解析、零磁盘 IO。
- `knowledge` 工具在自己的写入后同步刷新一次（`refresh_now`），因此能看到自己的写入；`/symbol-refs` 用同一引擎和缓存（`query_symbols`）。
- 无常驻索引时（CLI、单元测试）用同一引擎做一次性全量计算（`compute_once`），规则相同。
- 正确性：`index_tests::incremental_equals_from_scratch_over_random_edits` 在 3 个种子 × 40 步随机编辑（改源文件、删文件、改节点、提交、大小写改名、新增子目录节点）上逐项比对增量结果与从零全量结果。

## 10. API / WS 契约

### 10.1 `GET /api/workspace/knowledge`

- 认证：与其它 `/api/*` 相同，`Authorization: Bearer <token>`（服务配置了 token 时）。失败：`401`，空 body。
- 永远立即返回，不在请求内计算。冷启动期间返回 `ready:false` 的快照（`version` 0 为空快照，`version` 1 起为带 `checking` 的预览）。
- 成功：`200`，body 为 `{"ok": true, "data": <Snapshot>}`。
- 错误：除 401 外没有业务错误码；索引内部失败不会让接口报错，而是继续返回最后一份成功发布的快照（版本不再增长）。网络/5xx 时前端保留上一份快照。

`Snapshot`：

```json
{
  "version": 12,
  "epoch": "4f2c-192a3b4c5d6",
  "ready": true,
  "root": "knowledge",
  "visibility": "public",
  "head": "9fceb02d0ae598e95dc970b74767f19372d61af8",
  "folders": [
    { "id": "内核", "name": "内核", "parent_id": null },
    { "id": "内核/子", "name": "子", "parent_id": "内核" }
  ],
  "unknown": [
    { "path": "内核/子/readme.md", "name": "readme.md", "folder_id": "内核/子" }
  ],
  "nodes": [
    {
      "path": "内核/seq.md",
      "id": "seq",
      "key": "seq",
      "status": "enabled",
      "invalid_status": null,
      "summary": "序号分配",
      "body": "正文 markdown（声明块之后）……",
      "x": 120.0, "y": 40.0, "w": null, "h": null,
      "extras": ["owner : andy"],
      "folder_id": "内核",
      "relations": ["other"],
      "severity": "error",
      "issues": [
        {
          "severity": "error",
          "code": "line_out_of_range",
          "message": "Lines 900-901 are outside \"src/a.rs\", which has 3 lines.",
          "ref_key": "range:src/a.rs\u0000900-901",
          "args": { "path": "src/a.rs", "lines": "900-901", "line_count": 3 }
        }
      ],
      "refs": [
        {
          "ref_key": "sym:src/a.rs\u0000impl Store › fn save",
          "kind": "symbol",
          "key": null,
          "path": "src/a.rs",
          "symbol": "impl Store › fn save",
          "lines": null,
          "raw": "[@ file=\"src/a.rs\" symbol=\"impl Store › fn save\"]",
          "status": "ok",
          "severity": null,
          "target_path": "src/a.rs",
          "args": { "path": "src/a.rs", "symbol": "impl Store › fn save" },
          "message": ""
        },
        {
          "ref_key": "range:src/a.rs\u0000900-901",
          "kind": "range",
          "key": null,
          "path": "src/a.rs",
          "symbol": null,
          "lines": "900-901",
          "raw": "[@ file=\"src/a.rs\" lines=\"900-901\"]",
          "status": "line_out_of_range",
          "severity": "error",
          "target_path": "src/a.rs",
          "args": { "path": "src/a.rs", "lines": "900-901", "line_count": 3 },
          "message": "Lines 900-901 are outside \"src/a.rs\", which has 3 lines."
        }
      ]
    }
  ]
}
```

字段语义：

| 字段 | 说明 |
|---|---|
| `version` | 单调递增（u64）。只在内容变化时 +1；同一进程内永不回退。 |
| `epoch` | 进程实例标识。epoch 变了说明服务重启、`version` 重新计数：前端必须接受新 epoch 的任何版本。 |
| `ready` | `false`：首次全量尚未完成，引用状态可能是 `checking`，`head` 为 `null`。 |
| `root` | `"knowledge"` / `".litecode/knowledge"` / `null`（没有知识库）。 |
| `visibility` | `knowledge` → `"public"`，其它 → `"private"`。 |
| `head` | 用于漂移的 HEAD commit oid；无仓库 / 未出生为 `null`。 |
| `nodes[].path` | 节点身份：相对知识根、`/` 分隔（KR-LAYOUT-1）。按路径排序。 |
| `nodes[].id` | 旧标识（键唯一时为键，否则为路径），仅为兼容 `knowledge` 工具保留；前端以 `path` 为准。 |
| `nodes[].key` | 声明键（去空白）；可能重复或不合法（见 issues）。 |
| `nodes[].status` | `enabled` / `disabled` / `pending`；`invalid_status` 为原文（不合法时）。 |
| `nodes[].body` | 声明块之后的正文原文。 |
| `nodes[].x/y/w/h` | 画布位置，未写为 `null`。 |
| `nodes[].extras` | 声明块里未识别的行，原样保留（往返写回用）。 |
| `nodes[].relations` | 正文里引用到的节点键（文档解析结果，与旧前端 `refs` 字段同义）。 |
| `nodes[].severity` | KR-VAL-1：`"error"` / `"warning"` / `null`。 |
| `nodes[].issues[]` | 该节点所有问题：引用问题（`ref_key` 非空，与 `refs` 中同 key 那条的 `status` 相同）+ 声明问题（`ref_key: null`）。 |
| `refs[]` | 每个 `ref_key` 一条（KR-VAL-2），按首次出现顺序。`kind` ∈ `node`/`file`/`symbol`/`range`；`key` 仅 node；`path` 为规范化路径（file/symbol/range，symbol **必有** path）；`symbol` 仅 symbol；`lines` 仅范围引用有值：规范化行号或格式错误时的原文；node / file / symbol 一律为 `null`（符号引用上多写的 `lines` 原文只出现在 `raw` 和 `args.lines`）；`raw` 为原文片段；`status` 见第 6 节；`severity` 为 `"error"`/`"warning"`/`null`；`target_path`：node 为目标节点路径（找不到为 `null`），其余为规范化路径；`args` 为结构化参数（`key`/`path`/`symbol`/`lines`，以及 `target_status`、`reason`、`line_count` 等）；`message` 为英文兜底文案（`ok`/`checking` 时为空串）。 |

不提供位置信息（行列 / 偏移）：前端按 `raw` 或本地抽取的 `ref_key` 对齐。

### 10.2 WS 推送 `knowledge/changed`

经已认证的 `/ws` 连接（JSON-RPC 通知）：

```json
{ "jsonrpc": "2.0", "method": "knowledge/changed",
  "params": { "version": 13, "epoch": "4f2c-192a3b4c5d6", "paths": ["内核/seq.md"] } }
```

- 只推 `version`（和 `epoch`）；`paths` 是内容有变化（含新增、删除）的节点路径，**仅作提示**，不是 diff。
- 订阅端丢了中间版本时推一次当前版本、`paths: []`。
- 前端：防抖后 `GET` 整份快照；只接受 `version` 更大（或 `epoch` 不同）的快照；WS 重连后无条件全量 `GET` 一次。

## 11. 前端需要做的改动

1. 面板加载、存在性、符号 / 漂移徽标全部改为 `GET /api/workspace/knowledge` + `knowledge/changed`；删除面板自己的 `/tree`、`/file` 逐目录扫描和 `/symbol-refs` 调用（F1/F2/F3/F5/F6/F7）。
2. 删除 `upgradeKnowledgeMarkdown` 的读取时写回（F4）。
3. `KnowledgeIssueCode` 增加：`symlink_escape`、`invalid_lines`、`unreadable_file`、`unsupported_language`、`ambiguous_symbol`、`line_out_of_range`；以 `status` / `code` 驱动展示，文案可本地化，`message` 只作兜底。
4. 节点身份改用 `path`；引用对齐用 `ref_key`（第 1.1 节公式），本地即时抽取用同一公式，状态查快照，查不到显示 `checking`。
5. `ready:false` 时显示「检查中」，不显示任何引用错误。
6. 契约表 v2：`extract` 每条带 `ref_key`；`resolve.expect` 的 `node` 一律是节点路径；`layout` 段覆盖文件夹树和未识别文件。前端测试可直接接入。
7. KR-SYM-6：本地抽取时，带 `symbol` 的引用即使写了 `lines` 也只算符号引用（`ref_key` 不含行号，不显示 `: 行号`）；快照里它是 `invalid_lines`、`args.reason` = `lines not allowed on symbol reference`、`lines: null`。前端插入 / 生成引用时不得在符号引用上写 `lines`（`markers.ts` 的生成函数需要相应收紧）。

## 12. 性能基线与金标

### 12.1 v1 基线（2026-10-11 实测，本仓 `knowledge/`：36 节点，52 个被引用路径，69 个不同符号 / 87 次符号检查，涉及 27 个源文件共 652 KB）

| 项目 | debug | release |
|---|---|---|
| `knowledge_status.note`（每条用户消息、首包前同步执行） | 9.85–10.0 s | 7.1–7.5 s |
| 其中 git 子进程 `git_show_at`（27 次） | 5.5 s | 5.6 s |
| 其中 tree-sitter `find_scope`（261 次解析） | 4.3 s | 1.5 s |
| `/symbol-refs` 等价调用（69 个查询） | 9.2 s | 6.8 s |

### 12.2 金标与 v2 实测

测法：本仓 `knowledge/`（36 节点、205 条引用、52 个被引用路径、27 个源文件），临时集成测试直接调用生产函数（已删除）。release 为 opt-level "z"、关闭 LTO、codegen-units 16 的近似；debug 为默认 dev。增量与 HEAD 项在「知识库 + 被引用文件」的临时副本（自建 git 仓库）上测，不碰真实工作区，每步都断言增量结果 == 从零全量结果。

| 指标 | 金标 | release 实测 | debug 实测 |
|---|---|---|---|
| 知识库对 `turn_to_first_request` 的贡献（暖态，`knowledge_status.note`，500 次） | release ≤ 5 ms，debug ≤ 10 ms（p95） | p50 0.0006 ms，p95 0.0007 ms，max 0.014 ms | p50 0.004 ms，p95 0.004 ms，max 0.037 ms |
| 每轮 git 子进程 / tree-sitter 解析 | 0 / 0 | 0 / 0（500 次 note 前后计数器不变；索引代码无 `Command`） | 同左 |
| 单个被引用文件改动后的增量重算（最大文件 `session.rs` 205 KB） | release ≤ 150 ms | p50 45.6 ms，max 47.8 ms | p50 157 ms，max 180 ms |
| 节点 `.md` 改动后的增量重算 | — | p50 0.9 ms | p50 2.6 ms |
| HEAD 变化后的漂移重算 | ≤ 300 ms | 单文件提交 46 ms；全部 27 个被引用文件同时变化 142 ms | 199 ms / 501 ms |
| 冷启动全量建索引（后台） | release ≤ 1 s，debug ≤ 3 s，不阻塞首轮 | 284–307 ms；`start()` 0.14 ms 返回（`ready:false`），294 ms 后就绪 | 784–1058 ms；898 ms 就绪 |
| 后台 2 s 全量 stat 兜底（暖态） | — | p50 2.8 ms | p50 6.0 ms |
| `GET` 服务端时间（取快照 + 序列化 137 KB JSON） | ≤ 20 ms | p50 1.9 ms，p95 2.6 ms | p50 7.8 ms，p95 13.4 ms |
| 正确性 | 契约表 100%；增量 == 全量 | 契约表 30 extract + 54 resolve + 1 layout 全过；随机 3×40 步与基准各步均相等 | 同左 |

冷启动解析次数：36 个 `.md`、27 个源文件各读一次、tree-sitter 54 次（27 份工作区 + 27 份 HEAD，各一次）、HEAD blob 27 个。

说明：
- turn 开始时若有**已入队**的相关变更（例如刚保存了被引用文件），最多等 50 ms，超时用上一份快照；无关路径在入队前就被过滤，不会让 turn 等待。
- 新鲜度 = watcher 300 ms 防抖 + 增量重算（release 实测 ≤ 50 ms）。
- 用户消息里带引用时的上下文展开（`reminder::mentions`）属于「发送时快照」，仍读一次被引用文件并解析（只在消息含符号 / 范围引用时发生），不计入上表的知识状态路径。