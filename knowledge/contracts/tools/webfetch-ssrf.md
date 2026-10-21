```node
node : webfetch-ssrf
status : pending
summary : webfetch 出站门：只许公网 http(s)；拒真私网/回环/CGNAT；放行 198.18 fake-ip（TUN）。
x : 2394
y : 918
w : 480
h : 503
domain : tools-permission
```

**准入：** A6。`validate_public_http_url` / `resolve_and_validate_public_http_url` 是 webfetch 的出站安全门：非 http(s)、缺主机、真私网/回环/link-local/ULA/CGNAT `100.64/10` 应拒绝。这是工具局部防护（不替代工作区沙箱/权限 floor）；门漏了会把内网当公网去拉。

**WP-203（TUN / fake-ip）：** `198.18.0.0/15`（RFC 2544 benchmarking，常见 Clash/Surge/Mihomo fake-ip）**不再**当作非公网拒绝。主机名解析到该段可过门；连接仍钉本趟解析 IP（`resolve_to_addrs`，ISSUE-021）。**坚持拒** RFC1918 / loopback / unspecified / link-local / `fe80` / unique-local / `100.64/10`。字面公网 IP 行为不变。

依据：[@ file="src/tools/webfetch.rs" symbol="fn validate_public_http_url"]、[@ file="src/tools/webfetch.rs" symbol="fn is_non_public_ip"]、[@ file="src/tools/webfetch.rs" symbol="fn is_ipv4_shared"]、[@ file="src/tools/webfetch.rs" symbol="impl Tool for WebFetchTool › fn call_inner"]。
父节点：[@ key="tool-implementation-contract"]。
相关：[@ key="tool-authorization"]。
基线：b38ebc4（WP-203 实现树）
**核实结论：与 WP-203 定稿对齐。**
- 出站门只许 http(s)；拒真非公网；**放行 198.18/15** —— [@ file="src/tools/webfetch.rs" symbol="fn is_non_public_ipv4"]（不再调用 benchmarking 分类进门）。
- 每跳重校验 + 钉 IP —— [@ file="src/tools/webfetch.rs" symbol="fn fetch_hops"]（redirect Policy::none）。
- 回归：`webfetch::tests::ssrf_*`（含 `ssrf_allows_benchmarking_fake_ip_range`、`ssrf_shared_cgnat_still_non_public`）；mapped/fe80/100.64 仍拒。系统代理路径仍可能由代理解析。
