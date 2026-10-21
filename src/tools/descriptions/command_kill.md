Stop a `run_command` job in this session from inside `run_code`. Pass the `job_id` returned with `kind: background` or `kind: promoted`.

The process tree is killed. The result is `kind: killed` with `exit_code` (integer or null), `output_file`, and `stderr_file`. This consumes the exit, so it does not also arrive as a user-stopped `reminder/bash_exit`.

An unknown `job_id`, or a job from another session, rejects `not_found`. This does not stop the human terminal.
