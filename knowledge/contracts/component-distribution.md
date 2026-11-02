```node
node : component-distribution
status : pending
summary : 外置组件经 litecode-components.json 声明，仅允许本仓 GitHub Releases URL，下载后校验 sha256。
x : 5364
y : 1278
domain : release-desktop
```

**准入：** A4 / A5 / A6。模型与 Linux tar 的身份、版本范围、大小与哈希写在清单里；`ComponentManager` 流式下载、失败可重试、校验不过不落盘；URL 前缀必须是 `github.com/itissika/litecode/releases/download/v…`，`LITECODE_COMPONENTS_BASE` 可指本地/替代源供离线开发。坏了是装到篡改包或失败拖垮无关功能。

依据：[@ file="desktop/src/components.ts" symbol="class ComponentManager"]、[@ file="desktop/src/components.ts" symbol="fn parseManifest"]、[@ file="desktop/src/components.ts" symbol="fn releaseDownloadPrefix"]、[@ file="scripts/build_components.mjs"]。
上层：[@ key="desktop-shell"]。
相关：[@ key="slim-packaging"]。
