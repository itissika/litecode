# 第三方声明

LiteCode 以 MIT 许可发布（见仓库根目录 `LICENSE`），并再分发/依赖以下第三方作品：

英文版：[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 模型权重（随仓再分发）

- **IBM Granite Embedding `granite-embedding-97m-multilingual-r2`** — Apache-2.0，版权归 IBM Corp.
  本仓库再分发的是其 **WOQ 量化（派生）版本，不是 IBM 官方权重**。来源与修改声明见
  `models/ibm-granite/granite-embedding-97m-multilingual-r2/README.md`，许可证全文见同目录 `LICENSE`。
  原模型：<https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2>

## 运行时依赖

- **ONNX Runtime**（Rust crate `ort`）— MIT
- **Hugging Face tokenizers**（Rust crate `tokenizers`）— Apache-2.0

## 桌面端（Electron）

- **Electron** — MIT
- **electron-builder** — MIT

各依赖的完整许可证文本见其自带的 LICENSE/NOTICE 文件（cargo registry / `node_modules`）。
