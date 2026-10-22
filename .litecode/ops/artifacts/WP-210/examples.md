# WP-210 examples — A–K before/after（cl100k_base token 计数）

说明：before 取自 e2a2e46 的输出格式，after 取自本包实现的输出格式；样例为代表性片段（与对应单测断言一致），用于对照形状与 token 量级。

| 项 | before tok | after tok | Δ |
|---|---:|---:|---:|
| A 失败保留部分结果 | 24 | 121 | +97 |
| B 运行时异常 line/column/栈 | 24 | 66 | +42 |
| C 超预算结构化截断 | 35 | 49 | +14 |
| D 成功 logs | 22 | 36 | +14 |
| E key 顺序/表格列 | 26 | 28 | +2 |
| F run stdout/stderr/shell | 39 | 55 | +16 |
| G 统一结果契约（read 缺文件） | 35 | 23 | -12 |
| H read 降噪 | 24 | 9 | -15 |
| I Code 内 fs 只读探针 | 24 | 29 | +5 |
| J websearch top_k | 35 | 33 | -2 |
| K 知识库失效引用 | 11 | 19 | +8 |
| 合计 | 299 | 468 | +169 |

要点：A/B/D/F 为增加信息量的有意增长（失败可定位、成功 logs 可见）；C/E/G/H/J 为降噪或结构化；J 去掉 provider 原文 text 后在真实查询下节省最多（原文常 500–1000 tok，样例已压缩）。

## A 失败保留部分结果

before（24 tok）:
```
Error: run_code failed: exception · 2 succeeded, 1 failed · 41ms
Error: edit failed
```
after（121 tok）:
```
run_code failed: exception · program: failed · tool calls: 2 ok / 1 failed · 41ms · line 6, column 3 · throw new Error("edit failed");
completed tool calls before the failure:
#1 read path=src/a.rs → ok: {"ok":true,"path":"src/a.rs","lines":["fn main() {}"]}
#2 grep pattern=TODO → ok: {"ok":true,"hits":[{"path":"src/a.rs","line":3,"hit":"// TODO"}]}
#3 edit path=src/a.rs → failed(no_match)
```

## B 运行时异常 line/column/栈

before（24 tok）:
```
Error: run_code failed: exception · 0 succeeded, 0 failed · 3ms
Error: bad state
```
after（66 tok）:
```
run_code failed: exception · program: failed · tool calls: 0 ok / 0 failed · 3ms · line 3, column 13 · throw new Error("bad state");
stack:
  at boom (program.js:3:13)
  at <main> (program.js:5:1)
```

## C 超预算结构化截断

before（35 tok）:
```
notice: result truncated to fit 8000 tokens
{"hits":[{"path":"src/a.rs","line":1,"hit":"x"},{"path":"src/b.rs","li
```
after（49 tok）:
```
notice: result over budget; arrays cut (keep 2)
{"hits":[{"path":"src/a.rs","line":1,"hit":"x"},{"path":"src/b.rs","line":9,"hit":"y"},"… 998 more"]}
```

## D 成功 logs

before（22 tok）:
```
run_code ok · 1 succeeded, 0 failed · 5ms
logs omitted (2 lines)
```
after（36 tok）:
```
run_code ok · program: ok · tool calls: 1 ok / 0 failed · 5ms

logs:
[log] hello 42
[error] warn
```

## E key 顺序/表格列

before（26 tok）:
```
hits[2]{hit,line,path}:
  TODO,3,src/a.rs
  FIXME,7,src/b.rs
```
after（28 tok）:
```
hits[2]{path,line,hit}:
  src/a.rs,3,TODO
  src/b.rs,7,FIXME
```

## F run stdout/stderr/shell

before（39 tok）:
```
{"id":"j1","exit_code":1,"output":"built\n[stderr]\nwarning: unused\n\udcd6\udcd0\udcce\udcc4"}
```
after（55 tok）:
```
{"id":"j1","exit_code":1,"output":"built\n","stderr":"warning: unused\n中文","shell":"bash (MINGW64)","message":"MSYS converts /c-style args to Windows paths; quote or use //c to keep them"}
```

## G 统一结果契约（read 缺文件）

before（35 tok）:
```
Error: run_code failed: exception · 0 succeeded, 1 failed · 4ms
uncaught ToolError not_found from call #1 read path=missing.txt
```
after（23 tok）:
```
// tools.read resolves:
{"ok":false,"reason":"not_found","message":"missing.txt does not exist"}
```

## H read 降噪

before（24 tok）:
```
Showing lines 1-3 of 3. Continue with offset=4.
[dropped undefined: 1]
```
after（9 tok）:
```
Line 2 truncated to 1500 chars
```

## I Code 内 fs 只读探针

before（24 tok）:
```
// 需 tools.glob + tools.read 两次调用
await tools.glob({pattern:"src/*"})
```
after（29 tok）:
```
fs.ls("src") → [{"name":"a.rs","type":"file","size":120}]
fs.exists("Cargo.toml") → true
```

## J websearch top_k

before（35 tok）:
```
{"text":"1. Rust book - https://doc.rust-lang.org/book/ The Rust Programming Language ... 2. ... (provider raw text ~800 tokens)"}
```
after（33 tok）:
```
{"ok":true,"results":[{"title":"The Rust Book","url":"https://doc.rust-lang.org/book/","snippet":"The Rust Programming Language"}]}
```

## K 知识库失效引用

before（11 tok）:
```
`src/tools/bash.rs` › `fn execute`
```
after（19 tok）:
```
`src/tools/run.rs` › `impl Tool for RunTool` › `fn execute`
```
