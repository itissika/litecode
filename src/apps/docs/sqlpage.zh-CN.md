# SQLPage

把 SQL 查询变成轻量的本地 Web 应用。托管安装会下载固定版本的 SQLPage Windows 包，并在工作区里创建起始站点。

- 页面是 `.litecode/apps/sqlpage/site` 下的普通 `.sql` 文件。`index.sql` 是首页；新增 `reports.sql` 后访问 `/reports.sql` 即可增加页面。
- 站点使用 SQLite 数据库：`.litecode/apps/sqlpage/sqlpage.db`。
- 服务只监听本机回环地址；启动时 LiteCode 会校验这一点。
- 运行期间可以直接修改 SQL 文件；开发模式会在每次请求时重新读取。
- 卸载会删除下载的运行时，但保留站点和数据库。
