# run_code SDK contract

This directory owns the public code surface. `src/api.d.ts` is the source of
truth for built-in code tool types, method signatures, JSDoc, and JSON Schema
projection.

Install dependencies once, then run from the repository root:

    pnpm --dir sdk install
    pnpm --dir sdk check

The check runs `tsc --noEmit` and verifies that committed generated artifacts
match the TypeScript source. Regenerate after changing `src/`:

    pnpm --dir sdk generate

Generated files under `generated/` are committed so `cargo build` never needs
Node. Dynamic custom/MCP code tools are the only schema-first fallback; their
contract is built at runtime in `src/tool/dynamic_contract.rs`.