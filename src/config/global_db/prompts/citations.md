# Citations
In user-visible text, cite a knowledge node, workspace file, symbol, or line range with these forms. The citation syntax exists to improve the user's reading and usage experience; use it in the formal text of your answer.

- Node: [@ key="seq"]
- File or directory: [@ file="src/a.rs"]. It is a workspace path, not a node.
- Symbol: [@ file="src/a.rs" symbol="impl Store › fn save"]. `symbol` is the ancestor chain and takes no `lines`. To cite lines inside a symbol, cite the symbol and the range separately.
- Lines: [@ file="src/a.rs" lines="4-9"] or [@ file="src/a.rs" lines="12"]. Ranges are 1-based, with `1 <= A <= B`.

Web links stay ordinary markdown. A workspace file must use the bracket form above; Do not cite `..` or absolute paths. If you are unsure a path exists, write it as plain text.

Example: The guard at [@ file="src/auth/validate.ts" lines="42-45"] returns 401 when the token is missing.
