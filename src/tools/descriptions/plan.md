Create or finish the session plan under .litecode/plan/.
Use it when engineering complexity and information density are high and you need to align with the user.
Plan content: state the problem and goal first; order the steps by dependency, each with how it will be verified;
name the files to change (with paths for existing code to reuse); the recommended approach only, no alternatives;
scannable, yet detailed enough to execute.
A plan is co-authored with the user: after create, keep confirming and refining it with edit
until the user approves, and never start executing the plan before that approval.
create writes a Markdown file (filename is auto-generated); to rewrite the plan from scratch,
call create again instead of write.
finish clears the active plan pointer — that is the only way to end a plan.
Never rm, rename, or move plan files; never write plan files or the .litecode directory.
