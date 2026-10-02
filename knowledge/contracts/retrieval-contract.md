```node
node : retrieval-contract
status : pending
summary : 检索契约：文件/会话为真源，索引可重建；命中按证据排序、水合验证，复制品不冒充原件。
x : 1224
y : 700
w : 398
h : 720
```

**主张：** 检索召回已有事实的证据；索引与回声副本不冒充事实源。

- 真源恒在：code 来自工作区文件、session 来自会话日志；索引、向量与排名都是可重建的派生数据，命中回真源水合、复核身份后才算证据。
- 硬排除与回声：agent 检索硬排除活跃 session 当前 Surface 上的行（被 compact 遮蔽的旧行仍可召回）；回声按调用关系结构化判定——复制品结果整条剔除、调用意图保留。
- 排序先证据后偏好：先按匹配证据与角色，再偏好调用者会话家族与较新的对话；展示分数不参与排序。
- 滞后是允许的：索引更新不阻塞查询，滞后只许少召回；已删除或已剔除的行不得成为有效证据。
- 预算是视图：输出是 token 预算内的证据视图，不承诺穷尽全量历史。

上层：[@ id="featured-tools" label="特色工具"]、[@ id="engine-design" label="engine 设计"]。
依据：[@ file="src/engines/session_search/echo.rs" label="回声剔除"]、[@ file="src/engines/session_search/ranking.rs" label="排序契约"]。
