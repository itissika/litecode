# Third-Party Notices

LiteCode is released under the MIT license (see `LICENSE` in the repository root).
It redistributes or depends on the following third-party works:

Chinese version: [THIRD_PARTY_NOTICES.zh-CN.md](THIRD_PARTY_NOTICES.zh-CN.md)

## Model weights (redistributed in this repository)

- **IBM Granite Embedding `granite-embedding-97m-multilingual-r2`** — Apache-2.0, © IBM Corp.
  This repository redistributes a **WOQ-quantized (derivative) build**, not the official
  IBM weights. See `models/ibm-granite/granite-embedding-97m-multilingual-r2/README.md`
  for the source and modification details, and the `LICENSE` file in the same directory
  for the license text.
  Upstream: <https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2>

## Runtime dependencies

- **ONNX Runtime** (Rust crate `ort`) — MIT
- **Hugging Face tokenizers** (Rust crate `tokenizers`) — Apache-2.0

## Desktop (Electron)

- **Electron** — MIT
- **electron-builder** — MIT

The full license texts are available in each dependency's own LICENSE/NOTICE files
(cargo registry / `node_modules`).
