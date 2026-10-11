```node
node : webfetch-ssrf
status : pending
summary : webfetch 出站门：只许公网 http(s)；私网/回环/共享址拒绝。
x : 2394
y : 918
w : 480
h : 503
domain : tools-permission
```

**准入：** A6。`validate_public_http_url` 是 webfetch 的出站安全门：非 http(s)、缺主机、回环与私网段应拒绝。这是工具局部防护（不替代工作区沙箱/权限 floor）；门漏了会把内网当公网去拉。已知 IPv4 映射 IPv6 等缺口记 issue，不在本节点扩写。

依据：[@ file="src/tools/webfetch.rs" symbol="fn validate_public_http_url"]、[@ file="src/tools/webfetch.rs" symbol="fn is_non_public_ip"]、[@ file="src/tools/webfetch.rs" symbol="fn is_ipv4_shared"]、[@ file="src/tools/webfetch.rs" symbol="impl Tool for WebFetchTool › fn call_inner"]。
父节点：[@ key="tool-implementation-contract"]。
相关：[@ key="tool-authorization"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b；Batch3 已修 ISSUE-012）
- 出站门只许 http(s) 且拒绝非公网 —— [@ file="src/tools/webfetch.rs" symbol="fn validate_public_http_url"]；分类见 [@ file="src/tools/webfetch.rs" symbol="fn is_non_public_ip"]（含 IPv4-mapped、`is_unicast_link_local`、198.18/15）。
- 每跳重校验 —— [@ file="src/tools/webfetch.rs" symbol="impl WebFetchTool › fn fetch"]（redirect Policy::none）。
- 回归：`webfetch::tests::ssrf_*`（含 mapped/fe80/198.18）；ISSUE-011 测试改写。残余 DNS 重绑定见 ISSUE-021（suspect）。
