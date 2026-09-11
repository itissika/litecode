# Litecode workspace contract

## 禁止高危 git 操作（stash / reset --hard / checkout -- / clean / restore 他人文件等）

原因：本仓库常有多个 agent 并行工作，工作区存在大量未提交改动。stash/reset 等操作会静默卷走或丢弃其他 agent 的进行中工作，且难以恢复。如确有必要，必须先向用户确认。
