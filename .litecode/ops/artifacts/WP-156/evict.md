# WP-154 冷启动：无 root 逐出页缓存（供 IDE / 前端复用）

> 磁盘回滚后重写；脚本在 /workspace/wp156-recover/mincore.py。

## 原理
- `posix_fadvise(fd, 0, 0, POSIX_FADV_DONTNEED)`：让内核丢弃该文件的**干净**页缓存。普通用户对自己能打开的文件就能调用，不需要 root，也不需要 `drop_caches`。
- 脏页（已写入、还没回写）不会被丢弃，所以要先 `fsync(fd)` 回写，再调 fadvise。刚编完的二进制、刚复制的索引或语料尤其要注意。
- 用 `mincore()` 核实：以 `PROT_READ, MAP_SHARED` mmap 文件（不触发读盘），逐页查询是否驻留。读数为 0 = 不在页缓存中。
- 局限：只逐出列出的文件。共享库（libc、libstdc++ 等）、目录项/inode 缓存不受影响；被其他进程 mmap 且正在使用的页可能逐不掉，这时读数不为 0，如实记录即可。真正的冷启动可能更慢，报告里要注明逐出了哪些。

## 要逐出的范围（每次冷启动前）
1. 被测二进制（如 `target/release/litecode`）。
2. 模型目录，递归：`models/ibm-granite/granite-embedding-97m-multilingual-r2/`。
3. 工作区目录，递归，包括 `.litecode/`（sessions.db、session-index、代码索引）。
4. 本次 run 的 HOME 目录，递归；有 web dist 的话也要逐出。

## 命令
```bash
M=/workspace/wp156-recover/mincore.py
python3 $M check  BIN MODEL_DIR WS_DIR HOME_DIR          # 逐出前读数（可选）
python3 $M evict --verify BIN MODEL_DIR WS_DIR HOME_DIR; echo exit=$?
```
输出：`{"files":N,"resident_pages":R,"total_pages":T,"still_resident":{path:[驻留页,总页]}}`。目录会自动递归，符号链接跳过。
**核实标准**：`resident_pages == 0`、`still_resident` 为空、退出码为 0。不为 0 时把 `still_resident` 原样附上，并注明"该部分页缓存仍热"。

单行版（逐个文件，只逐出、不核实）：
```bash
python3 -c 'import os,sys;[ (lambda fd:(os.fsync(fd),os.posix_fadvise(fd,0,0,os.POSIX_FADV_DONTNEED),os.close(fd)))(os.open(p,os.O_RDONLY)) for p in sys.argv[1:]]' FILE...
```

## 时序
- 逐出要放在所有准备写入（复制语料、种子、engines.json、编译）**之后**、启动进程**之前**。逐出后立即启动，中间不要读这些文件（`cat`、`cp`、算哈希都不行）。
- 每次 run 都记录逐出后的读数，和计时数据放在一起。
- 之前在 box 上实测（overlay 文件系统，无 root）：sessions.db 6241→0 页，onnx 87→0 页；WP-156 的所有 run 中，本次 run 自己的文件逐出后全部为 0 页。
