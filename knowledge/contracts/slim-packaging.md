```node
node : slim-packaging
status : pending
summary : Windows 发布默认瘦 NSIS：不内嵌 embed 模型与 Linux tar；portable 不再作为发布形态。
x : 5598
y : 1278
domain : release-desktop
```

**准入：** A1。`package_win.ps1` / `assemble_product` / `desktop/package.json` 默认 slim；`-Full` 只许本地离线。DirectML 因 sidecar 启动硬依赖可留在包内。坏了是安装包再次膨胀，或 CI 把不该嵌入的资源打回去。

依据：[@ file="scripts/package_win.ps1"]、[@ file="desktop/package.json"]、[@ file="scripts/assemble_product.ps1"]。
上层：[@ key="desktop-shell"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 默认 slim —— [@ file="scripts/package_win.ps1"]；`extraResources` 仅 sidecar（无 models/linux）—— [@ file="desktop/package.json"]。
