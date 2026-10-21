Read a `run_command` job in this session from inside `run_code`. Pass the `job_id` returned with `kind: background` or `kind: promoted`.

`wait_sec` is an integer from 0 to 120 and defaults to 0. `0` returns the current snapshot and does not wait. A positive value waits until the process exits or the time runs out.

`kind: running` means the process is still going. The job is left running. `kind: exited` means it finished, with `exit_code` an integer or null. Both kinds include `stdout` and `stderr` windows (`head`, `tail`, `bytes`, `truncated`) plus `output_file` and `stderr_file`. An exited result consumes that exit, so the same exit does not also arrive as `reminder/bash_exit`.

An unknown `job_id`, or a job from another session, rejects `not_found`. Cancelling this wait rejects `cancelled` and does not kill the job. Stop the job with `tools.command_kill`.
