# granite-embedding-97m-multilingual-r2 (LiteCode quantized bundle)

This directory contains an ONNX Runtime weight-packed (WOQ) quantization of
[`ibm-granite/granite-embedding-97m-multilingual-r2`](https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2),
quantized and redistributed by the LiteCode project.

Chinese version: [README.zh-CN.md](README.zh-CN.md)

## Source

- Original model: <https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2>
- Upstream license: Apache-2.0 (Copyright IBM Corp.; full text in `LICENSE` in this directory)
- Architecture: ModernBERT, 384-dim embeddings, 32,768-token context, 200+ languages

## Modification notice (Apache-2.0 §4)

These are **modified weights (a derivative work), not the official IBM release**;
IBM does not endorse this build.

- Weights are packed with RTN (no calibration): embedding MatMuls are int8 (block 128);
  attention/FFN MatMuls are int4 (block 128, MatMulNBits). This is **weight-only
  quantization**: graph activations stay FP — `accuracy_level=1` runs the MatMulNBits
  internal activation path in **fp32**. The `dtype: bfloat16` in `config.json` refers to
  the upstream safetensors weights, not the ONNX graph activations.
- Runtime execution (CPU EP): the packed weights are **dequantized block-by-block to
  fp32** for each GEMM and the activations are multiplied at fp32 precision; quantization
  saves disk size, load bandwidth and resident memory, not compute precision.
- Full quantization metadata: `artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json`
  (algorithm, bits, block size, op counts, sha256)
- The quantization toolchain is **not shipped** in this repository (internal eval
  pipeline); `scripts/bundle_embed_model.sh` copies the artifacts into the product tree.

## Files

| File | sha256 |
|------|--------|
| `config.json` | `933b3105f0a4688d762a2742d3aa103335fd08d8888bc74d52a28aef35494337` |
| `tokenizer.json` | `d230784319ccf89032f23ef3a06ae5aab6a4b9d73be32de0a0989c793e463d27` |
| `1_Pooling/config.json` | `2d0a5053a404b23e265843108c7013580890de5af4cb0b3933b06468d535052f` |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx` | `efb976f475b0ceb23d68d230bc9ce474cd3290178d08c1c323c18df00d5295c5` (matches `SOURCE.json`) |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx.data` | `7b5cd1b732a218f82b57f4ec2070c12e75f34e72b8a7a4cb257802eb1a08c22a` |
| `artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json` | — (quantization audit record) |

Notes: `config.json` declares the original BF16 dtype; the shipped weights are
actually int4/int8-packed after quantization.

## Usage

LiteCode loads this model via ONNX Runtime (`ort`) for local code/session semantic
search (`src/engines/code_search`, `src/engines/session_search`); no network required.
