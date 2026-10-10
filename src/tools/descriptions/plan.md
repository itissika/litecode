Create or finish the session plan under .litecode/plan/.
Use when engineering complexity and information density are high and you need to align with the user.
create: pass a short summary of the plan you intend to write. The user is asked to Approve or Reject (optional opinion). On approve, a stub plan file is created and you receive its path plus any opinion - then write the full plan body with edit. On reject, no file is created.
Plan body (via edit): state the problem and goal first; order steps by dependency, each with how it will be verified; name the files to change (with paths for existing code to reuse); the recommended approach only, no alternatives; scannable, yet detailed enough to execute.
A plan is co-authored with the user: after create, keep confirming and refining it with edit until the user approves, and never start executing the plan before that approval.
To rewrite the plan from scratch, call create again instead of write.
finish clears the active plan pointer - that is the only way to end a plan.
Never rm, rename, or move plan files; never write plan files or the .litecode directory with write - use edit on the path returned by create.
