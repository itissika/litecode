```node
node : context-media
status : pending
summary : 媒体只属于当次请求：模型看一眼，下一次请求就换成文字；库里留着可追溯，但不再进模型视野。
x : 5094
y : 954
domain : harness-core
```

**准入：** A1 / A3。一条图片或视频的寿命只有一次请求：产生它的那次请求里模型能看到，之后的请求里只剩文字占位，永不重现。理由有两条——重复计费，以及旧画面继续影响后面的判断。库里不受这个限制：媒体本身要留着可回看、可追溯，但把历史媒体重新塞回模型等于回退。

现状：工具产生的媒体已经做到「一次」；用户自己贴的媒体仍按整个回合保留，与「当次请求」还差一步，这是要收敛的地方。

依据：[@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]、[@ file="src/context_pipeline/compact.rs" symbol="struct MechanicalPrunePlan"]。
父节点：[@ key="context-view"]。
基线：666540c

**核实结论：按意图订正，实现有一处未收敛。**（基线 666540c）
- 工具媒体一次 —— 以最后一个模型产出为界，之前的一律换文字 [@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]。
- 用户媒体整个回合 —— 起点是最后一个最终回答之后，所以一轮里的每一步都还看得到 [@ file="src/context_pipeline/media_budget.rs" lines="1-10"]；与「当次请求」的差距在这里。
- 库里侧：机械裁剪把过期媒体在持久面换成占位，原始行保留 [@ file="src/context_pipeline/compact.rs" symbol="struct MechanicalPrunePlan"]。
- 压缩不再单独剥媒体：原 `strip_media_for_summary` 随「压缩复用主请求」删除。
