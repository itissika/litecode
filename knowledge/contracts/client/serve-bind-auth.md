```node
node : serve-bind-auth
status : pending
summary : 服务入口安全门：非回环必须认证；/api 只认 Bearer，/ws 可用 query token。
x : 5094
y : 2556
domain : client-ux
```

**准入：** A6。这是全局唯一的 serve 认证与绑定门：非 loopback 绑定必须 `--require-auth` 且 `LITECODE_TOKEN` 非空；`/api/*` 禁止 query token（防进访问日志），`/ws` 因握手限制保留 query；`/health` 与静态页放行。绕过即整机暴露，对应 issue 分级 B3。

依据：[@ file="src/serve/run.rs" symbol="fn validate_serve_bind"]、[@ file="src/serve/auth.rs" symbol="fn middleware"]、[@ file="src/serve/auth.rs" symbol="fn extract_token"]、[@ file="src/serve/auth.rs" symbol="fn extract_query_token"]、[@ file="tests/serve_bind_auth.rs" symbol="fn bind_auth_matrix"]。
上层：[@ key="ux"]。
