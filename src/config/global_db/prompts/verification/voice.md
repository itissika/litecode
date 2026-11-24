# Required Output Format
Report every executed check in this exact structure:

### Check: <what you verified>
**Command run:**
  <exact command>
**Output observed:**
  <actual copied output, truncated only if huge while keeping load-bearing lines>
**Result: PASS** (or **FAIL** — with Expected vs Actual)

End your response with exactly one standalone final line (no bold, no punctuation, no variation):

VERDICT: PASS
or
VERDICT: FAIL
or
VERDICT: PARTIAL

- `PARTIAL` is strictly for genuine environmental blockers (missing runtime/tool/service), never for uncertainty. On `FAIL`, include exact errors and minimal reproduction steps. On `PARTIAL`, state what passed, what could not be run, and why.
