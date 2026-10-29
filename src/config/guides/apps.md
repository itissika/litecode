## Apps（本地应用）

Apps 面板把应用分成两类：

- **内置应用**：LiteCode 能一键安装、卸载、托管安装目录和缓存的，才进内置目录。内置应用带离线说明和官方仓库链接，面板里直接安装/启动/卸载。
- **外置应用**：LiteCode 不安装、不卸载、不管目录和缓存。它只负责按注册表里的声明启动进程或提供静态站点，然后嵌进面板。外置应用要么是 LiteCode 的推荐项（尚未注册），要么是用户/agent 写进注册表的具体应用。

远程工作区不显示 Apps。

### 面板结构

- **已安装**：内置应用完成安装的；外置应用只要注册过就算已安装。这里是一个平铺列表，不再分类。
- **未安装**：尚未安装的内置应用 + 尚未注册的外置推荐，两者都按用途分类。
- 内置未安装项点开是详情，有安装按钮和完整说明。
- 外置推荐没有安装按钮，详情里提供一句可复制给 agent 的话，例如「帮我在 LiteCode 中安装 ComfyUI（仓库地址）」；人类把这句话交给 agent 即可。人类不需要自己写注册表。

### 注册表

- 工作区：`.litecode/apps.json`
- 全局：Windows 为 `%LOCALAPPDATA%\litecode\apps.json`，macOS / Linux 为 `~/.local/share/litecode/apps.json`
- 两层是**追加**关系，不是覆盖；合并后 ID 必须唯一。重复 ID 会丢弃低优先级条目并报错。
- 内置目录始终可用，不需要写注册表。注册表里与内置同 ID 的条目会被视为重复。

最小形状（外置进程）：

```json
{
  "version": 2,
  "apps": {
    "my-server": {
      "name": "My Server",
      "description": "A loopback-only local server.",
      "category": "general",
      "icon": "🛠️",
      "launch": {
        "kind": "process",
        "command": "pnpm",
        "args": ["dev", "--port", "{port}"],
        "cwd": "tools/server",
        "port": "auto",
        "url": "http://127.0.0.1:{port}/",
        "timeout_ms": 30000
      }
    }
  }
}
```

静态站点：

```json
{
  "version": 2,
  "apps": {
    "my-site": {
      "name": "My Site",
      "description": "A static site built in this workspace.",
      "category": "document",
      "icon": "🌐",
      "launch": { "kind": "static", "dir": "dist", "spa": false }
    }
  }
}
```

### 字段（必填 / 选填）

必填只放 LiteCode 运行所必需的字段：

- `name`、`description`、`category`、`icon`、`launch`。

选填字段没填就不显示：

- `description_i18n`：按语言覆盖一句话摘要（如 `"zh-CN"`），缺失时回退 `description`。
- `repo`：官方仓库链接，必须是 https URL。
- `docs`：可选的官方文档链接，必须是 https URL。
- `single_instance`：为 true 时，同一应用在其他工作区已运行则拒绝再启动。
- `permissions`：只接受 `microphone`、`camera`、`clipboard-read`、`clipboard-write`、`fullscreen`。

`category` 取值：`data` / `analysis` / `document` / `design` / `image` / `video` / `audio` / `task` / `general`。
`icon` 用 emoji 或文本；工作区注册表可用 `"file:assets/icon.png"`（相对工作区，最大 64 KiB）。

### 启动

- `launch.kind: "process"`：
  - `command` 原样执行，LiteCode 不改写命令或参数（`npx` 就是 `npx`）。绝对路径直接用；带路径分隔符的相对路径按 `cwd` 解析；裸命令先查 `PATH`（Windows 含 `PATHEXT`），再查 `cwd`。
  - `cwd` 默认 `"{workspace}"`；可写相对工作区的路径或绝对路径。目录必须已存在，LiteCode 不会替你创建。
  - `args`、`cwd`、`env` 和 `url` 支持 `{workspace}`、`{port}`。`{appdir}`、`{data}` 是内置应用专用，外置注册表使用会被拒绝。
  - `port` 为 `"auto"`（默认）或固定数字；`{port}` 会替换成实际分配的端口。
  - `url` 必须是 `http` 且指向 `127.0.0.1`、`localhost` 或 `::1`。
  - `ready_path`（可选，须以 `/` 开头）：除端口可达外，还要求该路径返回 2xx/3xx。
  - `timeout_ms` 默认 30000，最大 3600000；超时报错并停止进程。
  - 启动后 LiteCode 会从本机 LAN 地址回连端口；若可达，立即杀掉进程并报 `non-loopback interface`。
- `launch.kind: "static"`：`dir` 指向工作区里的目录（相对工作区或绝对路径），由 LiteCode 在随机 loopback 端口提供；`spa: true` 启用单页回退。目录不存在或缺少 `index.html`（非 SPA）会直接报错。

### 内置应用与数据

- 内置应用由 LiteCode 管理安装目录：程序在 `<全局数据>/apps/<id>/app/`，安装日志和标记也在同一父目录。
- 工作区数据在 `<workspace>/.litecode/apps/<id>/`；应用私有数据在安装目录的 `data/`。卸载只删程序与安装日志，**保留数据**。
- 内置应用的离线说明随应用展示；外置应用没有离线说明，只有摘要、仓库链接和可选的 `docs` 链接。

### 外置应用边界

- 自定义应用没有 `install`、`data`、`summary`、`homepage`、`readme_url`、体积这些字段。`version: 1` 仍可读，但这些字段会被忽略并给出弃用警告；`cwd: "app"` 或自定义静态缺少 `dir` 会成为该应用自己的错误。
- LiteCode 不为外置应用安装依赖、不接管工作目录、不保存日志、不清理缓存。应用自己的数据、日志和依赖都在它自己的位置。
- Docker、远程主机、任意 URL 代理都不在这个能力范围；进程只监听 loopback。

### 信任

内置应用直接可信。外置应用第一次启动前，人类需要在面板里确认信任预览。信任按工作区和应用记录在 `<全局数据>/app-trust.json`，绑定 `launch`、`single_instance` 和 `permissions` 的哈希；这些字段改了就需要重新确认。Agent 可以写注册表，但不能代替人类点信任或启动。

### Agent 处理外置推荐

面板里的外置推荐只是元数据，不在注册表里，也没有可执行的 launch。人类把复制框里的话交给你时：

1. 先读 `litecode_workspace guide apps`，确认字段与边界。
2. 询问或确认人类希望把运行时装在哪里；不要假定路径，也不要写进 LiteCode 的内置安装目录。
3. 安装/准备该应用的依赖和运行时（由你按应用自己的方式完成）。
4. 用真实路径写注册表：`cwd` 指向能运行的位置，`command` 用实际可执行文件或 PATH 上的命令，端口交给 `{port}`。
5. 让人类在 Apps 面板确认信任；不要代替人类信任或启动。

`litecode_workspace apps` 会列出未注册的外置推荐（名称、ID、摘要、仓库），以及已注册应用的状态、URL 和安装/运行错误。这个命令只读，不会安装、信任或启动任何东西。
