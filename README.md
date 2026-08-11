# Alter

一个面向 **Hyprland + Wayland** 的快速启动器、全局搜索和剪贴板历史工具。

当前版本是可以日常使用的 Wayland 启动器：

- `Super + Space` 唤起/隐藏搜索浮层
- `Super + Shift + C` 直接打开剪贴板专用搜索
- 搜索并启动 `.desktop` 应用
- 使用 `plocate` 搜索文件（没有可用数据库时回退到 `fd`）
- 读取现有 Clipse 的文本、图片和文件历史；没有 Clipse 时监听文本剪贴板并保存最近 500 条记录
- 进入 `c` / `clip` 剪贴板范围后，选择记录即可在右侧预览完整内容，图片结果显示大图；普通搜索不占用预览区域
- `a `、`f `、`c ` 前缀分别限定应用、文件和剪贴板
- 输入算式时提供安全的内置计算器，按 Enter 复制结果
- 搜索 `settings` / `设置` 或按齿轮进入 Alter 设置
- 应用结果显示 `.desktop` 图标；没有图标时使用系统 fallback 图标
- 通过 StatusNotifierItem 在 Waybar 托盘常驻，菜单可打开、进入设置或退出
- 剪贴板默认保留 30 天，可在设置中调整为 1–3650 天
- 支持 Google、Bing、DuckDuckGo Web 搜索及可选搜索建议
- 支持文件动作面板、关键词 Workflow、自定义 Snippets 和使用频率学习排序
- 全部操作支持键盘：Enter、↑、↓、Tab / →、Esc

## 构建

### Arch Linux（AUR）

AUR 包名为 `alter-launcher`（`alter` 已被其他项目占用）：

```bash
yay -S alter-launcher
```

安装后可直接运行 `alter --daemon`，并按下文配置 Hyprland 快捷键。

### 从源码构建

系统依赖（Arch/EndeavourOS）：

```bash
sudo pacman -S --needed base-devel rust gtk4 gtk4-layer-shell plocate fd wl-clipboard curl xdg-utils
```

在项目目录构建并运行：

```bash
cargo run --release
```

二进制文件位于 `target/release/alter`。

维护者推送与 `Cargo.toml` 版本一致的 `v*` tag 后，GitHub Actions 会在 Arch
容器中验证 AUR 包、生成 `.SRCINFO` 并发布到 `alter-launcher`。

## Hyprland 快捷键

当前配置已经把 `Super+Space` 绑定给 Alter，并用 `Super+Shift+C` 直接进入剪贴板范围；在其他机器上可添加下面两行：

```ini
bind = SUPER, SPACE, exec, /path/to/alter/target/release/alter --toggle
bind = SUPER SHIFT, C, exec, /path/to/alter/target/release/alter --clipboard
```

然后重新加载配置：

```bash
hyprctl reload
```

`Super+Space` 会在同一个进程中切换浮层；`Super+Shift+C` 会复用同一窗口并自动填入
`c ` 搜索范围，不会不断创建新窗口。

当前 Hyprland 配置已经加入下面这一行；如果你在其他机器部署，可以按需添加：

```ini
exec-once = /path/to/alter/target/release/alter --daemon
```

`--daemon` 会在后台等待快捷键，不会显示窗口。

## 与 Alfred 的功能差距

Alter 目前聚焦于 **Hyprland + Wayland 的本地启动与搜索**。下表按当前代码实际支持情况比较，避免把 Linux 上暂时不可用的 macOS 专属能力算成已实现：

| 能力 | Alter | Alfred | 说明 |
| --- | --- | --- | --- |
| 全局启动器、模糊搜索 | 已有 | 已有 | Alter 依赖 Hyprland 的 `Super+Space` 绑定 |
| 快捷键与搜索作用域配置 | 部分 | 已有 | Alter 的快捷键在 Hyprland 配置中修改；`Super+Shift+C` 可直达剪贴板，设置页可控制本地、Web、Workflow、Snippet 和学习排序，`a/f/c` 可快速限域 |
| 应用启动、文件搜索 | 已有 | 已有 | Alter 使用 `.desktop`、`plocate` / `fd` |
| 搜索范围扩展 | 部分 | 已有 | Alter 已支持 Web、Workflow 和 Snippets；浏览器书签、联系人等原生数据源仍缺失 |
| 剪贴板历史 | 文本/图片/文件 | 文本/图片/文件及更丰富操作 | Alter 可读取 Clipse，支持右侧完整预览、图片预览、固定和隐藏；自带监听器目前只采集文本 |
| 计算器 | 已有 | 已有 | Alter 支持基础四则、括号和常用运算 |
| 设置与主题 | 已有基础设置 | 完整偏好、主题和同步 | Alter 支持深浅主题与各搜索模块开关，目前只保存本地配置 |
| 托盘常驻 | 已有 | 菜单栏常驻 | Alter 使用 StatusNotifierItem，适配 Waybar |
| Web 搜索、建议 | 已有 | 已有 | 使用 `? query`、`web query` 或 `g/b/ddg query` |
| 文件操作/Universal Actions | 基础已有 | 已有 | Alter 支持打开、定位、复制路径/URI 和经确认移入回收站，尚无多步动作链 |
| Snippets / 文本展开 | 基础已有 | 已有 | Alter 支持关键词与 `{query}` 替换并复制内容；Wayland 下不注入按键自动粘贴 |
| Workflow / 插件系统 | 基础已有 | 已有 | Alter 支持 JSON manifest、关键词触发、argv 命令、cwd/env/icon、Script Filter 和多个命名动作，尚无可视化编辑和复杂编排 |
| 学习排序、搜索历史 | 已有 | 已有 | Alter 按明确选择的频率和时间提升常用结果，可在设置中关闭 |
| Shell 命令、URL、书签、联系人等扩展 | 部分 | 已有（部分为 macOS 集成） | URL 和 argv 命令可通过 Workflow 扩展，书签/联系人等尚无原生索引 |
| 配置导入/导出、跨设备同步 | 暂无 | 已有 | Alter 目前是单机配置 |

仍值得继续补齐的三项是：

1. Workflow 的图形化编辑和复杂动作链（Script Filter 与多个命名动作已有基础版本）；
2. 浏览器书签、历史与密码管理器等可选数据源；
3. Snippets 的输入法级安全文本展开、配置导出和同步。

### Web 搜索

在搜索框中使用下面任一种显式前缀即可生成浏览器动作，按 Enter 后由
`xdg-open` 打开默认浏览器：

```text
? wayland layer shell       # 使用默认 DuckDuckGo
web gtk4 css                 # 使用默认 DuckDuckGo
g rust gtk                  # Google
b linux launcher             # Bing
ddg clipboard manager        # DuckDuckGo
```

查询会按 RFC 3986 规则进行 UTF-8 百分号编码，输入不会经过 shell。Google、
Bing 和 DuckDuckGo 都带有轻量建议接口；网络或 `curl` 不可用时，建议列表会
静默为空，不影响本地搜索。

可在 `~/.config/alter/web-searches.json` 添加或覆盖搜索模板。文件内容是 JSON
数组，每项至少包含 `id`、`name`、`keywords` 和 `url_template`，URL 中必须有
`{query}`：

```json
[
  {
    "id": "baidu",
    "name": "百度",
    "keywords": ["bd", "baidu"],
    "url_template": "https://www.baidu.com/s?wd={query}",
    "suggestion_template": "https://suggestion.baidu.com/su?wd={query}"
  }
]
```

也支持更易手写的 `~/.config/alter/web-searches.conf`，每行格式为：
`id|名称|关键词,别名|搜索 URL|建议 URL`（建议 URL 可省略）。

Web 搜索和联网建议可分别在 Alter 设置中关闭。关闭建议后不会发起建议请求，
自定义 URL 仍由 `xdg-open` 交给系统默认浏览器。

### 文件动作

在应用或文件结果上按 `Tab` 或 `→` 进入动作页，使用 `↑` / `↓` 选择并按
Enter 执行；按 `←`、Backspace 或 Esc 返回搜索。当前动作包括：

- 打开应用、文件或目录；
- 在文件管理器中打开所在目录；
- 复制完整路径或安全转义后的 `file://` URI；
- 将文件或目录移入系统回收站。

“移入回收站”会显示包含目标路径的二次确认，并通过 `gio trash` 执行。应用的
`.desktop` 条目不会提供删除动作，避免误删系统或 Flatpak 的启动项。

### Workflow / 插件

每个 Workflow 是 `~/.config/alter/workflows/` 下的一个 JSON 文件。下面的例子在
输入 `gh rust gtk` 后生成一个结果，按 Enter 会打开 GitHub 搜索：

```json
{
  "id": "github-search",
  "name": "搜索 GitHub",
  "description": "在默认浏览器中搜索仓库和代码",
  "keyword": "gh",
  "command": [
    "xdg-open",
    "https://github.com/search?q={query}"
  ],
  "icon": "web-browser",
  "enabled": true
}
```

`keyword` 也可写成 `keywords` 数组。`{query}` 可用于命令参数、`cwd` 和 `env`
的值，但不能替换可执行文件本身。命令始终按 argv 数组直接启动，不使用
`sh -c`；一个损坏的 manifest 也不会阻止其他 Workflow 加载。

#### Script Filter

需要根据输入动态列出候选项时，可以在 manifest 中加入 `"script_filter": true`。
脚本只会在用户明确输入 Workflow 关键词时运行（不会因为普通模糊搜索而执行），
并且有 800 ms 超时、256 KiB 输出上限和最多 50 个结果。脚本可以输出 Alfred
兼容的 JSON：

```json
{
  "id": "project",
  "name": "项目搜索",
  "keyword": "p",
  "script_filter": true,
  "command": ["/path/to/project-filter", "{query}"],
  "actions": [
    {
      "title": "打开项目",
      "subtitle": "使用默认程序打开",
      "command": ["xdg-open", "{arg}"],
      "icon": "document-open"
    },
    {
      "title": "复制路径",
      "command": ["wl-copy", "{arg}"],
      "icon": "edit-copy"
    }
  ]
}
```

输出可以是 `[ { "title": "…", "subtitle": "…", "arg": "…", "icon": "…" } ]`
或 `{ "items": [ … ] }`；小脚本也可以每行输出 `标题<TAB>参数`。选择结果后，
`actions` 会显示在候选项的 `Tab / →` 动作页，均支持 `{query}`（原始输入）和
`{arg}`（当前候选参数）；直接按 Enter 会执行第一个动作。旧版单一 `action`
字段仍兼容，并会被转换成默认动作。没有动作时，Alter 会把 `arg` 作为下一次
Workflow 参数执行同一个命令。所有命令仍使用 argv，不经过 shell。脚本来自用户
自己的配置，启用前请确认命令路径和权限。

### Snippets

可在 `~/.config/alter/snippets.json` 保存一个对象或数组。例如输入
`sig Alter` 后，结果会把 `{query}` 替换为 `Alter`，按 Enter 将完整文本复制到
剪贴板：

```json
[
  {
    "id": "signature",
    "name": "邮件签名",
    "keywords": ["sig", "签名"],
    "content": "谢谢，\n{query}",
    "enabled": true
  }
]
```

也可使用 `~/.config/alter/snippets.conf`，每行格式为
`id|名称|关键词,别名|内容`；内容支持 `\n`、`\r`、`\t`、`\\` 和 `\|` 转义。
Snippet 只复制展开结果，不读取 shell，也不会模拟键盘输入。

### 剪贴板高级操作

- 输入 `c `、`clip ` 或 `clipboard ` 进入剪贴板范围后，右侧预览面板会显示完整文本；Clipse 的图片文件会显示可滚动的大图，长文本可在面板内滚动查看。普通搜索会隐藏该面板，让结果列表保持完整宽度。
- `Ctrl+Shift+P`：固定或取消固定当前剪贴板结果；固定项以星标显示，并不会因
  保留期清理而过期。
- Delete：在 Alter 中隐藏当前剪贴板结果，不会修改或重写 Clipse JSON。
- Clipse 中带 `filePath` 的图片会显示缩略图，文件结果使用对应文件图标；按
  Enter 会把图片 MIME 或文件 URI 写回 Wayland 剪贴板。
- 保留期默认 30 天，可在设置中改为 1–3650 天。

固定和隐藏状态保存在 Alter 自己的 SQLite metadata 表中。没有 Clipse 时，Alter
自带的 `wl-paste --watch` 监听器目前只保存文本；图片和文件历史需要 Clipse。

### 学习排序

Alter 会在用户明确按 Enter 执行结果时记录使用次数和最近使用时间，并给常用的
应用、文件、剪贴板、Web 搜索、Workflow 和 Snippet 增加有限的排序分数。频率
加成有上限，时间加成会逐步衰减并在 30 天后归零，因此不会永久压过更匹配的
新结果。空查询也会评估完整的应用索引，因此常用应用即使原本排在首屏之后仍
可以被提升。可在设置中关闭“学习排序”。

联系人、Safari/浏览器书签、iTunes 等 Alfred 的 macOS 专属集成不属于 Alter 当前 Wayland 目标；X11、GNOME、KDE 适配也暂不在范围内。

## 数据位置

- 设置：`~/.config/alter/settings.conf`
- Web 模板：`~/.config/alter/web-searches.json` 或 `web-searches.conf`
- Workflow：`~/.config/alter/workflows/*.json`
- Snippets：`~/.config/alter/snippets.json` 或 `snippets.conf`
- 剪贴板、固定/隐藏 metadata 和使用统计：`~/.local/share/alter/history.sqlite3`

如果检测到你现有的 `clipse -listen`，Alter 会直接读取
`~/.config/clipse/clipboard_history.json`，不会再启动重复的监听器。没有
Clipse 时，第一次打开 Alter 会启动自己的 `wl-paste --watch` 子进程；若手动结束
Alter，监听进程也会被清理。

剪贴板内容会以本地明文形式保存在 Clipse JSON 或 Alter SQLite 中；如果机器上有
密码、令牌等敏感内容，请使用 Clipse 的清理/暂停功能或后续为 Alter 增加排除规则。

## 已知限制

- Alter 自带的剪贴板监听器只采集文本；图片和文件历史目前从 Clipse 读取。
- 文件搜索依赖 `plocate` 数据库；数据库未更新时，新文件可能需要等待索引更新。
- 剪贴板和 Snippet 结果只会写回剪贴板，不会自动向前一个窗口发送 Ctrl+V；
  Wayland 安全模型不允许普通应用无条件向其他客户端注入按键。
- 目前针对 Hyprland/Wayland，未实现 X11、GNOME 或 KDE 专用适配。
