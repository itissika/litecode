# WP-154 冷启动：无 root 逐出页缓存（供 IDE / 前端复用）

## 原理
- `posix_fadvise(fd, 0, 0, POSIX_FADV_DONTNEED)`：通知内核丢弃该文件的**干净**页缓存。普通用户对自己能打开的文件即可调用，不需要 root，也不需要 `drop_caches`。
- 脏页（刚写入、尚未回写）不会被丢弃 → 先 `fsync(fd)` 回写再 fadvise。刚编完的二进制、刚复制的索引或语料尤其要注意。
- `mincore()` 核实：以 `PROT_READ, MAP_SHARED` mmap 文件（不触发读盘），逐页查询是否驻留。读数 0 = 该文件不在页缓存中。
- 局限：只逐出列出的文件。共享库（libc、系统 .so）、内核自身缓存、目录项/inode 缓存不受影响；被其他进程 mmap 且正在使用的页可能留下，这时 mincore 读数不为 0，如实记录。真正的冷启动可能更慢，报告中需注明逐出范围。

## 要逐出的范围（每次冷启动前）
1. 被测二进制：例如 `target/release/litecode`（release 由 IDE 在 W1 提供）。
2. 模型目录（递归）：`models/ibm-granite/granite-embedding-97m-multilingual-r2/`（含 `artifacts/*.onnx`、tokenizer、config）。
3. 工作区目录（递归，含 `.litecode/`：sessions.db、session-index、code 索引等）。
4. 若有：web dist（`LITECODE_WEB_DIST`）、HOME 下的全局配置目录。

## 命令
脚本：`/workspace/litecode/.litecode/ops/artifacts/WP-155/mincore.py`（目录自动递归，跳过符号链接）
```bash
cd /workspace/litecode
M=.litecode/ops/artifacts/WP-155/mincore.py
BIN=target/release/litecode
MODEL=models/ibm-granite/granite-embedding-97m-multilingual-r2
WS=/path/to/workspace

# 逐出前读数（可选）
python3 $M check $BIN $MODEL $WS
# 逐出（内部先 fsync 再 DONTNEED），--verify：仍有驻留页则退出码 1
python3 $M evict --verify $BIN $MODEL $WS; echo exit=$?
```
输出 JSON：`{"files":N,"resident_pages":R,"total_pages":T,"still_resident":{path:[驻留页,总页]}}`。
**核实标准**：`resident_pages == 0` 且 `still_resident` 为空（退出码 0）。不为 0 时，把 `still_resident` 列表原样附进结果，并注明"该部分页缓存仍热"。

单行版（单个文件，无 mincore 核实）：
```bash
python3 -c 'import os,sys;[ (lambda fd:(os.fsync(fd),os.posix_fadvise(fd,0,0,os.POSIX_FADV_DONTNEED),os.close(fd)))(os.open(p,os.O_RDONLY)) for p in sys.argv[1:]]' FILE...
```

## 时序要求
- 逐出必须在被测进程**启动前**、且在所有准备写入（复制语料、种子索引、编译）**之后**做；逐出后立刻启动，中间不要读这些文件（`ls -l` 没问题，`cat`/`cp`/哈希不行）。
- 每次运行都在日志里记一次逐出后的读数，与计时数据放在一起。
- 已在 box 上验证：sessions.db 6241→0 页，onnx 87→0 页，small_repo 56→0 页（overlay 文件系统，无 root）。
