# granite-embedding-97m-multilingual-r2（LiteCode 量化版）

本目录存放 [`ibm-granite/granite-embedding-97m-multilingual-r2`](https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2)
的 ONNX Runtime 权重打包（WOQ）量化版本，由 LiteCode 项目量化并随仓库再分发。

英文版：[README.md](README.md)

## 来源

- 原模型：<https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2>
- 原许可：Apache-2.0（版权归 IBM Corp.，全文见本目录 `LICENSE`）
- 架构：ModernBERT，384 维向量，上下文 32,768 token，支持 200+ 语言

## 修改声明（Apache-2.0 第 4 条）

**这是 IBM 官方权重的修改版（派生作品），不是 IBM 官方发布，IBM 未背书本构建。**

- 权重采用 RTN（无校准）打包量化：embedding 层 MatMul 为 int8（block 128）；
  attention/FFN 层 MatMul 为 int4（block 128，MatMulNBits）。这是**仅权重量化（weight-only）**：
  图激活保持浮点——`accuracy_level=1` 使 MatMulNBits 内部激活路径以 **fp32** 运行。
  `config.json` 中的 `dtype: bfloat16` 指上游 safetensors 权重的精度，不是 ONNX 图激活精度。
- 运行时执行（CPU EP）：打包权重在每次 GEMM 前**按 block 反量化回 fp32**，激活以 fp32 精度相乘；
  量化省的是磁盘体积、加载带宽和常驻内存，而不是计算精度。
- 完整量化元数据见 `artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json`
  （算法、位宽、block 大小、算子统计、sha256）
- 量化工具链**未随本仓库分发**（位于 LiteCode 内部 eval 工程）；
  `scripts/bundle_embed_model.sh` 负责将产物拷入产品树

## 文件清单

| 文件 | sha256 |
|------|--------|
| `config.json` | `933b3105f0a4688d762a2742d3aa103335fd08d8888bc74d52a28aef35494337` |
| `tokenizer.json` | `d230784319ccf89032f23ef3a06ae5aab6a4b9d73be32de0a0989c793e463d27` |
| `1_Pooling/config.json` | `2d0a5053a404b23e265843108c7013580890de5af4cb0b3933b06468d535052f` |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx` | `efb976f475b0ceb23d68d230bc9ce474cd3290178d08c1c323c18df00d5295c5`（与 SOURCE.json 记录一致） |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx.data` | `7b5cd1b732a218f82b57f4ec2070c12e75f34e72b8a7a4cb257802eb1a08c22a` |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json` | —（量化审计记录） |

说明：`config.json` 声明的是原始 BF16 dtype；量化后随仓权重实际为 int4/int8 打包。

## 使用

LiteCode 通过 ONNX Runtime（`ort`）加载该模型，用于本地代码/会话语义检索
（`src/engines/code_search`、`src/engines/session_search`），无需联网。
