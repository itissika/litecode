# WP-065 汇总（前端中层）

父包：WP-063 · 子包：WP-066（desktop）已验收 done  
明细：`WP-063-desktop.md`  
Issue：**ISSUE-015**（release 构建卫生）  
范围：仅 `scripts/assemble_product.{sh,ps1}`。未碰 ISSUE-004 / 后端 / 产品 UI。未 commit。

## 完成对照

| 条款 | 落点 | 抽样 |
|------|------|------|
| release remap registry/git | assemble sh+ps1：`PROFILE!=debug` 幂等追加 `--remap-path-prefix=…/registry=/rustc/registry` 与 `…/git=/rustc/git` | assemble_product.sh/.ps1 |
| 发布链经 assemble | package_linux/win/local*/sidecar/signed-release 均经 assemble；yml 无需再写 | WP-063-desktop 链图 |
| 证据 | A：中层复跑 release/idempotent/preserve/debug 均绿；B：无现成 release 二进制，建议 CI strings 抽测 | WP-063-desktop.md |
| debug/dev 不动 | serve.sh 裸 release 故意未改；debug 不注入 | 同 |

## 建议 ISSUE-015

证据 A 齐、注入点正确；二进制 strings（B）留给 CI/签名产物抽测。建议 **closed**（gate/build-hygiene），或 GM 若要以 CI strings 为硬门则暂记 follow-up 不挡本包。

## 结论

ISSUE-015 前端/desktop 腿已落。请 GM 整包验收。未 commit。
