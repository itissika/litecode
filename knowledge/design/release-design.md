```node
node : release-design
status : pending
domain : release-desktop
summary : 发布形态：Windows 唯一瘦 NSIS；大资源按需从本仓 GitHub Releases 拉取，不用 CDN。
```

**准入：** A1 / A7。安装包体积与可重复发版是产品边界：默认包只带内核壳，模型与 Linux remote tar 外置；更新源锁死本仓 Releases，禁止第二套 CDN/镜像合约。坏了要么安装包膨胀失控，要么客户端拉到不可信源。

上层：[@ key="product-brief"]。
框架展开：[@ key="desktop-shell"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 默认瘦 NSIS —— [@ file="scripts/package_win.ps1"]、[@ file="desktop/package.json"]；按需组件契约见 [@ key="component-distribution"]。
