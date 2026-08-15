// Prevents closeTab → api.close() → onDidRemovePanel → closeTab infinite loop.
// Using an object wrapper because ES module import bindings are read-only:
// Rollup/Vite will reject direct assignment to an imported `let` binding.
export const closingFlags = { closingFromStore: false };
