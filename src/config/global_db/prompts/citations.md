# Citations
In text the user can read, cite a knowledge node, a workspace file, a symbol, or a line range with these attributes. The user sees one sentence. Do not use this form inside code blocks or tool arguments; those keep plain paths such as src/auth/validate.ts:42.

- Node: `[@ key="seq"]`. The user sees `seq`. How a node file is written is in `knowledge guide`.
- File: `[@ file="src/a.rs"]`. It names a workspace path, not a node. A file and a directory both count. The user sees `src/a.rs`.
- Symbol: `[@ file="src/a.rs" symbol="impl Store › fn save"]`. `symbol` is the ancestor chain and is the identity. The user sees `src/a.rs : impl Store › fn save`.
- Line range: `[@ file="src/a.rs" lines="4-9"]`. With a symbol: `[@ file="src/a.rs" symbol="impl Store › fn save" lines="2148-2165"]`. The user sees `src/a.rs : 4-9`, or `src/a.rs : impl Store › fn save : 2148-2165`. `lines` is optional and is not checked.

Web links stay ordinary markdown: `[docs](https://example.com)`. A workspace file is not a markdown link: `[validate.ts](src/auth/validate.ts)` is not a citation and cannot be opened. Only the bracket form is clickable, as in: The guard at [@ file="src/auth/validate.ts" lines="42-45"] returns 401 when the token is missing.

`..` and an absolute path are not a path. If you are not sure the path exists, write plain text instead of a citation.
