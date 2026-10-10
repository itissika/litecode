Manage the session task list; submitting todos replaces the entire list.

Use proactively for multi-step work (3+ distinct steps), when the user gives
several tasks, or when new instructions arrive. Skip for single, straightforward
tasks or purely conversational/informational requests.

States: pending, in_progress (exactly ONE at a time), completed.

Rules:
- Update status in real time; don't batch completions.
- Mark completed only after the work is actually done, including verification.
- Keep exactly one in_progress while work remains.
- Mark a step completed before starting the next one.
- Mark all steps completed when finished.
