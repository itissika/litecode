# wp156-recover（磁盘回滚后重建，10-15）
- session.md：按最终汇报数字重写；原始 summary.json 已丢失。
- evict.md、mincore.py：页缓存逐出方法与工具（与原版功能相同）。
- ws_client.mjs：模拟前端的打开顺序（A/B 两条路径），与原版逻辑相同；新增 WS_MODULE 环境变量。
- harness.py：计时 harness。工作目录改为 WP156_DIR（默认 /workspace/wp156-recover/run，不在仓库内写东西）；逐出范围显式包含本次 run 的 HOME 与 web dist。
- fresh_big.sh（C2：代码种子 + 语料 + engines_big.json）、fresh_big_c1.sh（C1：无种子）：大仓副本改为放在 $WP156_DIR/big。
- 没有重建 prime.py（建索引脚本）和 analyze.py（汇总脚本），需要时再补。
- 均已做语法检查（node --check / py_compile / sh -n）；没有起 serve，也没有编译。

## 早期快照恢复的文件

prime.py、analyze.py、method.md 是磁盘回滚后从早期快照恢复的版本，留作日后补测大仓 C2，可能不是最终版。
