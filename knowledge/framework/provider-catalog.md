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

**准入：** A3+A7。运行时只解析进程内嵌 `default_catalog.toml`，不读盘上的 catalog 文件；新增同协议厂商改种子即可。Rust 只拥有目录契约与每种 `EndpointKind` 一个 codec——按 provider id 分支或另拉厂商目录都算第二真源。设置提交不能改目录事实（需重启）。

依据：[@ file="src/provider_catalog/store.rs" symbol="fn embedded"]、[@ file="src/provider_catalog/schema.rs" symbol="enum EndpointKind"]、[@ file="src/types/death_list_gate.rs" symbol="fn death_list_provider_catalog_is_the_only_llm_source"]。
上层：[@ key="config"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 内嵌种子唯一来源 —— [@ file="src/provider_catalog/store.rs" symbol="fn embedded"]；[@ file="src/types/death_list_gate.rs" symbol="fn death_list_provider_catalog_is_the_only_llm_source"]。
- EndpointKind 选 codec —— [@ file="src/provider_catalog/schema.rs" symbol="enum EndpointKind"]；产品入口按目录选 codec：[@ file="src/llm/mod.rs" symbol="fn provider_from_model"]。
- 不是可写设置文档 —— config-commit 主张目录为构建事实；提交不能改目录。
