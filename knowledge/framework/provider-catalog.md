```node
node : provider-catalog
status : enabled
summary : provider 目录：构建内嵌种子是唯一来源；按 EndpointKind 选 codec；不是可写设置文档。
x : 1728
y : 684
w : 480
h : 449
domain : model-config
```

运行时只解析进程内嵌 `default_catalog.toml`，不读盘上的 catalog 文件；新增同协议厂商改种子即可。Rust 只拥有目录契约与每种 `EndpointKind` 一个 codec——按 provider id 分支或另拉厂商目录都算第二真源。用户想要什么provider和模型联系我，我再加，不给用户自由度，为啥：我加上会全测一遍，确保对话时能随便切，一点毛病都不会有

依据：[embedded](src/provider_catalog/store.rs#embedded)、[EndpointKind](src/provider_catalog/schema.rs#EndpointKind)、[death_list_gate.rs](src/types/death_list_gate.rs)、[provider_from_model](src/llm/mod.rs#provider_from_model)。
上层：[config](knowledge/framework/config.md)。
