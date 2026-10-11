```node
node : desktop-shell
status : pending
domain : release-desktop
summary : Electron 壳托管 sidecar 与远程能力；壳不发明产品真源，只解析路径、下载组件、转发 IPC。
```

**准入：** A3。真源在 sidecar（litecode.exe）与工作区；桌面壳负责进程寿命、IPC、Open Remote、组件落盘到 `%LOCALAPPDATA%/litecode/bundles`。壳若再持一份布局/会话/模型状态，会与 web 投影和 sidecar 抢权威。

依据：[@ file="desktop/src/sidecar.ts"]、[@ file="desktop/src/main.ts"]、[@ file="desktop/src/bundle-paths.ts" symbol="fn litecodeBundlesDir"]。
上层：[@ key="release-design"]。
契约展开：[@ key="slim-packaging"]、[@ key="component-distribution"]、[@ key="bundle-resolve"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 壳职责 —— [@ file="desktop/src/main.ts"]、[@ file="desktop/src/sidecar.ts"]；bundles 根 [@ file="desktop/src/bundle-paths.ts" symbol="fn litecodeBundlesDir"]。
