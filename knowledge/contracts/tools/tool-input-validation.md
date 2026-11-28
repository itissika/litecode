```node
node : tool-input-validation
status : enabled
summary : 参数先验形状再验语义；多传的参数只有「被拒」和「当没看见」两种下场，没有中间态。
x : 2124
y : 810
w : 434
h : 554
domain : tools-permission
```

**一句话：** 模型给的参数，先按 schema 验形状，再交给工具验语义；形状不对的调用连工具都进不去。

**关键点：多传的参数要么被拒，要么被忽略，没有「提醒一下」**

- schema 关死（`additionalProperties: false`）就直接拒，报 `unknown parameter 'x'`；schema 没关死，那些多出来的键就当没看见。以前那条「附一条 Warning 继续跑」的路已经删干净（[validate_object](src/tool/schema_validate.rs#validate_object)）。
- 形状这层读的是构建期生成的工具契约，code 模式与直接调用同一份；过了这层才轮到工具自己的 `validate_input`（[check_tool_input](src/tool/schema_validate.rs#check_tool_input)）。

**不变量：** 工具实现拿到的参数一定是先验过的——形状和语义两关过不去，就永远进不到实现里。

上游：[tool-call-contract](knowledge/contracts/tools/tool-call-contract.md)。
下游：[tool-authorization](knowledge/contracts/tools/tool-authorization.md)（校验通过才谈授权）、[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)（调用怎么收场）。
