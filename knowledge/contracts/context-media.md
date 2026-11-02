```node
node : context-media
status : pending
summary : 媒体只属于当次请求：模型看一眼，下一次请求就换成文字；库里留着可追溯，但不再进模型视野。
x : 5094
y : 954
domain : harness-core
```

**准入：** A1 / A3。一条图片或视频的寿命只有一次请求：产生它的那次请求里模型能看到，之后的请求里只剩文字占位，永不重现。理由有两条——重复计费，以及旧画面继续影响后面的判断。库里不受这个限制：媒体本身要留着可回看、可追溯，但把历史媒体重新塞回模型等于回退。

依据：[@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]、[@ file="src/context_pipeline/compact.rs" symbol="struct MechanicalPrunePlan"]。
父节点：[@ key="context-view"]。
