```node
node : theme-tokens
status : enabled
summary : 视觉单一来源：`--_dk-*` token 与 glassFill；颜色只传达状态，不散落硬编码。
x : 990
y : 810
w : 480
h : 383
domain : client-ux
```

**准入：** A1。主题与磨砂玻璃必须只有一处定义（`tokens.css` / `composerCard`），业务组件只消费 token；否则 agent 卡、权限卡、Engines 下载卡会各长一副面孔，违背 ux-design 的克制美学。坏了是视觉分裂，不是功能错，但会毁掉「安静克制」承诺。

依据：[@ file="web/src/theme/tokens.ts"]、[@ file="web/src/components/composerCard.ts"]、[@ file="web/src/theme/tokens.css"]。
上层：[@ key="ux"]。
相关：[@ key="ux-design"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一 token 定义 —— [@ file="web/src/theme/tokens.css"]；磨砂 [@ file="web/src/components/composerCard.ts"]。
