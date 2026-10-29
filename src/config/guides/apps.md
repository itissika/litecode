## Apps（本地应用）

Apps 面板列出内建目录和自定义注册表里的本地 Web 应用。应用安装在固定目录；数据默认落在**当前工作区**，工作区切换后跟着切换；`data: "app"` 的应用使用安装目录里的私有数据。远程工作区不显示 Apps。

### 注册表

- 工作区：`.litecode/apps.json`
- 全局：Windows 为 `%LOCALAPPDATA%\litecode\apps.json`，macOS / Linux 为 `~/.local/share/litecode/apps.json`
- 两层是**追加**关系，不是覆盖；合并后 ID 必须唯一。重复 ID 会丢弃低优先级条目，并在面板的注册表错误里显示。想覆盖同一个应用请使用新 ID。
- 内建目录始终可用，不需要写注册表。

最小形状：

```json
{
  "version": 1,
  "apps": {
    "my-notes": {
      "name": "My Notes",
      "description": "A tiny local notes UI.",
      "description_i18n": { "zh-CN": "一个极简的本地笔记界面。" },
      "category": "general",
      "icon": "📝",
      "install": { "kind": "none", "installed": true },
      "launch": { "kind": "static", "spa": false }
    }
  }
}
```

外部应用把文件放进安装目录的 `app/`，LiteCode 只负责启动和停止。非静态应用示例：

```json
{
  "version": 1,
  "apps": {
    "my-server": {
      "name": "My Server",
      "description": "Loopback-only local server.",
      "category": "general",
      "icon": "🛠️",
      "install": { "kind": "none", "installed": true },
      "launch": {
        "kind": "process",
        "command": "my-server.exe",
        "args": ["--port", "{port}"],
        "cwd": "app",
        "port": "auto",
        "url": "http://127.0.0.1:{port}/",
        "timeout_ms": 30000
      },
      "data": "workspace",
      "permissions": []
    }
  }
}
```

### 安装目录

- 内建 / 全局自定义：`<全局数据>/apps/<id>/`
- 工作区自定义：`<全局数据>/apps/ws/<workspace-hash>/<id>/`

目录内固定为 `app/`（程序）、`data/`（应用私有数据）、`logs/`、`run/` 和 `.litecode-app.json`。工作区作用域的数据在 `<workspace>/.litecode/apps/<id>/`。卸载只删程序和日志，保留数据。

### 字段与边界

- `description` 必填，用一句话说明应用做什么；`description_i18n` 可按语言覆盖这句话（如 `"zh-CN"`），界面按当前语言取，缺失时回退 `description`。
- `install.kind`：`steps` 由 LiteCode 下载 / 解压 / 执行安装步骤；`none` 表示外部安装。外部安装用 `installed: true` 声明由用户提供文件；LiteCode 仍会检查 `install.check`、静态 `index.html` 或启动命令是否存在，缺失时显示未安装。
- 自定义应用的每个 `download` 步骤必须写 `sha256`。内建目录可以不带哈希。
- `launch.kind`：`process` 启动子进程；`static` 由 LiteCode 在随机 loopback 端口提供 `app/` 静态文件。静态应用默认不启用 SPA 回退，需要时设 `spa: true`。
- 进程 URL 必须是 `http` 且指向 `127.0.0.1`、`localhost` 或 `::1`；支持 `{port}`、`{data}`、`{appdir}`、`{workspace}`。
- 进程必须只监听 loopback。启动后 LiteCode 会从本机 LAN 地址回连端口；若可达，立即杀掉进程并报 `non-loopback interface`。Remotion Studio 目前没有 host 参数，属于已知限制。
- 自定义图标用 `"file:assets/icon.png"`（相对工作区，最大 64 KiB），或直接用 emoji / 文本。
- `permissions` 只接受 `microphone`、`camera`、`clipboard-read`、`clipboard-write`、`fullscreen`。
- `readme_url` 只接受 `https://raw.githubusercontent.com/...`。
- Docker、远程主机、任意 URL 代理都不在这个能力范围；进程只监听 loopback。

### 信任

内建应用直接可信。自定义应用第一次安装或启动前，人类需要在面板里确认信任预览。信任按工作区和应用记录在 `<全局数据>/app-trust.json`，并绑定可执行字段的哈希；定义改了就需要重新确认。Agent 可以写注册表，但不能代替人类点信任、安装或启动。

### Agent 查看与排错

- 用 `litecode_workspace apps`（或 `apps <id>`）查看当前平台的全部内置和自定义应用：来源、安装方式、状态、运行中的 URL、数据目录、安装/运行错误。这个命令只读，不会安装、信任或启动。
- 内置应用（`source: builtin`）不需要注册表；ID 和安装方式以命令输出为准，不要凭记忆猜。
- 安装失败：面板详情会显示错误、日志路径和末尾几行；人类复制这段即可。Agent 也可以直接读输出里的 `install log`（`<安装目录>/logs/install.log`）和运行日志（`<安装目录>/logs/app.log`），以及 `install dir` 下的文件。
- 封装工作流：先用 `apps` 确认应用已安装或正在运行；运行中的应用会输出 `url`，端口是动态的，以输出为准；数据目录看 `data`。安装、信任、启动仍由人类在 Apps 面板完成。
