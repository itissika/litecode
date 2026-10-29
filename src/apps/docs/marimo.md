# marimo

Reactive Python notebooks in the browser. The managed install creates a virtual
environment inside the app directory and installs the pinned marimo version.

- The interpreter lives at `.venv` under the app directory.
- Notebooks and data belong to the workspace; LiteCode starts marimo there.
- The first start can take a few seconds while the environment warms up.
- Uninstall removes the app directory but keeps workspace notebooks.
