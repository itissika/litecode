```node
node : bundle-resolve
status : pending
summary : 组件查找顺序 env → bundles → sidecar → repo；打包后的应用不得回落到源码树路径。
x : 5094
y : 1278
domain : release-desktop
```

**准入：** A3 / A7。本地已有组件时行为不变；`devRepoRoot` 在 `app.isPackaged` 时必须返回空，release 二进制不得烘焙 `CARGO_MANIFEST_DIR` 式源码回退（Rust 侧同纪律归 ide-substrate）。坏了是构建机上「永远找到仓库模型、从不下载」。

依据：[@ file="desktop/src/bundle-paths.ts" symbol="fn resolveBundledModelDir"]、[@ file="desktop/src/bundle-paths.ts" symbol="fn devRepoRoot"]、[@ file="desktop/src/components.ts" symbol="fn resolveComponentSource"]。
上层：[@ key="desktop-shell"]。
相关：[@ key="component-distribution"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 查找序 + packaged 不回落 —— [@ file="desktop/src/bundle-paths.ts" symbol="fn resolveBundledModelDir"]、[@ file="desktop/src/bundle-paths.ts" symbol="fn devRepoRoot"]。
- 构建路径前缀卫生（第三方 panic）见 ISSUE-015（gate，不升产品）。
