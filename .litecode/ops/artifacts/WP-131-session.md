# WP-131 / Session：引擎进度 session 侧上报（seed）

接口与 GM 拍板见 WP-131-backend.md（冻结）。

## 范围
1. session refresh/rebuild 通过共享进度记录上报 state+progress（现状无 job.json，touch_code_job=false）。
2. chunks_done 取自 meta indexed_chunks；chunks_pending 精确，不估算。
3. 先切块得精确 chunks_total（允许 scanning，期间 total=null）。
4. 失败 → failed + error{engine:session,...}。
5. 复用 IDE 的共享记录类型：等 IDE 落类型或先与 IDE 约定签名。

## done_when
只刷 session 也显示 refreshing 且进度在走；done≤total 单调、完成相等；运行中 total=0 ≤ 一个上报周期；无估算；云端绿。
