```node
node : ui-system
status : pending
summary : UI 分层与复用契约：tokens 单一权威，primitives 只抽机制，两个真实消费者才提取，不造通用 Tree/Form。
```

LiteCode UI 的复用契约：tokens 是唯一视觉权威，primitives 只抽机制，产品语义留在消费方。

## 分层

tokens.css → components/ui primitives → patterns（menu / confirm / selection / dnd）→ product surfaces（FileTree、SessionList、GitPanel、Gallery、Knowledge）。

## 提取规则

- 至少两个真实消费者，且有行为测试，才提取。长得像不算。
- 抽焦点、键盘、portal、定位、提交/取消、drop 反馈；文件、会话、git、知识的业务语义留在消费方。
- 不做通用 `Tree<T>` / Form / Table；Button/Tag 继续用现有 CSS grammar，不额外包 React 组件。
- 新 primitive 必须用 token；禁止字面量颜色、阴影、z-index。
- 迁移保持现网视觉；唯一允许的可见变化是应用内 ConfirmDialog 替换 `window.confirm`。

## 主题

- 唯一源：`web/src/theme/tokens.css`；`@theme` 只做别名；`tokens.ts` 只做 JS bridge；desktop 由构建复制。
- `--_dk-*` 只能定义在 tokens.css（局部 hook / override 例外）；light override 必须完整。
- z ladder、字体、圆角、overlay shadow 已进 tokens；组件里不再写 `z-50` / `z-[10000]`。
- 例外：Shiki / Monaco 语法色；局部组件 hook 颜色/阴影；一次性宽度/间距不强行 token 化。
- 守卫：`web/src/theme/tokens.contract.test.ts`。

## 交互 canonical

- ContextMenu：唯一 cursor-anchored menu；键盘、焦点返回、pane/viewport clamp、popout 文档。
- ConfirmDialog + useConfirmDialog：每面板渲染，不设全局 host；禁止 `window.confirm`。
- ListRow、InlineRename、useDragSource/useDropTarget、applyListSelection、useAnchoredPanel 是各自机制的唯一入口。
- 浮层 portal 到触发元素的 `ownerDocument.body`；Dropdown/Popover 共用 useFloatingLayer，并共用 useAnchoredPanel 的定位状态与 pane resize 重算；视觉 preset 仍各自拥有。
- DnD：FileTree/Knowledge 本地实现暂时保留，kit 缺 external drop / 多格式；先补 kit 再迁移，不能直接删。
- 媒体库：grid/list 用 listbox + option 语义，多选走 Shift/Ctrl，键盘走方向键/Home/End/Ctrl+A/Ctrl+D/x，Enter/Space 打开，底部选择栏承担 bulk actions。搜索和排序是 view scope；hover preview 不是 quick look 的替代。
