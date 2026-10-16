# WP-158 重跑校验（当前树）
- 时间：2026-10-15 11:36–11:39 HKT，严格串行
- HEAD：9ce2ab77e8c42f7402c93e74786b1691697262a1（ops/team-framework），工作区有未提交改动（.litecode/ops/*、src/*.rs、tests/*.rs，web/desktop 源码无改动）
- 内存守护：.tmp/wp159/guard.sh 不存在，改为每秒采样 MemAvailable；最低 5.16 GiB（5408952 kB），未触发 2GiB 门槛
- node_modules：web/desktop 均存在，未跑 npm ci

| 步骤 | 命令 | 结果 |
|---|---|---|
| web tsc | npx tsc -b | PASS（0 输出） |
| web vitest | npx vitest run --maxWorkers=1 | PASS 1381/1381，176s |
| web knip | npx knip | exit 1（报告性）：导出 23 / 类型 21 / 文件 1 / devDep 0 / 未声明 0，与基线 23/21/1/0/0 一致 |
| desktop tsc | npx tsc -p tsconfig.json --noEmit | PASS（0 输出） |
| desktop test | npm test -- --test-concurrency=1 | PASS 140/140（49 suites） |

注：knip 不在 web devDependencies，npx 临时拉取 knip@6.41.0。
