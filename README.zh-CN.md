# Alter

[![项目主页](https://img.shields.io/badge/项目主页-访问-8b5cf6)](https://yuukidach.github.io/alter/) [![English README](https://img.shields.io/badge/English-README-blue)](README.md) [![许可证](https://img.shields.io/github/license/yuukidach/alter)](LICENSE)

Alter 是同时支持 **Linux/Wayland 与 Windows** 的快速启动器，提供应用和文件搜索、剪贴板历史、网页搜索，以及 Quick Links、Workflow、Snippet 等扩展。

![Alter 搜索界面](screenshots/alter-search-rounded.png)

## 功能

- 模糊搜索并启动 Linux `.desktop` 应用或 Windows“开始”菜单快捷方式。
- 使用 `plocate` 或 `fd` 搜索文件；Windows 上都不可用时使用内置搜索。
- 可选集成 [Clipse](https://github.com/savedra1/clipse)，浏览文本、图片和文件剪贴板历史，支持预览、固定和隐藏。
- 内置计算器，按 Enter 将结果复制到剪贴板。
- 使用 Google、Bing 或 DuckDuckGo 搜索网页。
- 自定义 Quick Links、Workflow 和 Snippet。
- 明暗主题和中英文界面；Linux 另有兼容 Waybar 的托盘图标。

## 依赖

### Linux

完整的浮层和托盘集成面向使用 Hyprland 的 Wayland Linux 桌面。

Arch Linux / EndeavourOS：

```bash
sudo pacman -S --needed base-devel rust gtk4 gtk4-layer-shell \
  plocate fd wl-clipboard curl xdg-utils
```

`plocate`、`fd` 和 Clipse 都是可选依赖。图片和文件剪贴板历史需要 Clipse；没有 Clipse 时，Alter 自带的监听器只保存文本。

### Windows

安装 [MSYS2](https://www.msys2.org/)，打开 **UCRT64** 终端并安装原生 Rust 与 GTK4：

```bash
pacman -Syu
pacman -S --needed git mingw-w64-ucrt-x86_64-rust \
  mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-pkgconf
```

## 安装

### AUR

```bash
yay -S alter-launcher
```

### 从源码构建

```bash
cargo build --release
./target/release/alter --daemon
```

Windows 请在 MSYS2 UCRT64 终端中构建：

```bash
cargo build --release
./target/release/alter.exe
```

从终端外启动时，请把 `C:\msys64\ucrt64\bin` 加入 `PATH`，其中包含 GTK 运行时 DLL。Windows 使用居中的普通窗口，关闭或执行结果后自动退出，可用偏好的 Windows 快捷键工具绑定 `alter.exe`；`--daemon` 和托盘图标目前仅支持 Linux。

## Hyprland 配置

启动一个后台实例，再绑定快捷键：

```ini
exec-once = /path/to/alter --daemon
bind = SUPER, SPACE, exec, /path/to/alter --toggle
bind = SUPER SHIFT, C, exec, /path/to/alter --clipboard
```

执行 `hyprctl reload` 重新加载配置。后台实例默认隐藏，快捷键调用时显示；后续调用会复用同一个窗口。

## 使用

| 输入 | 作用 |
| --- | --- |
| `a query` | 搜索应用 |
| `f query` | 搜索文件 |
| `c query` | 搜索剪贴板历史 |
| `? query` / `web query` | Google 搜索 |
| `g query` / `b query` / `ddg query` | Google / Bing / DuckDuckGo |
| `settings` 或 `Ctrl+,` | 打开设置 |

常用按键：

- `Enter`：启动、打开或复制当前结果
- `↑` / `↓`：移动选择
- `Tab` / `→`：打开应用、文件或 Workflow 的操作面板
- `←` / `Backspace`：返回搜索页
- `Ctrl+Shift+P`：固定或取消固定剪贴板记录
- `Delete`：在 Alter 中隐藏剪贴板记录
- `Esc`：隐藏浮层

命令行模式：

```text
alter --daemon       后台启动并常驻托盘
alter --toggle       切换已有实例的浮层
alter --clipboard    直接进入剪贴板搜索
alter --capture      从 stdin 读取一条文本并写入历史
```

## 配置文件

大多数选项可在 **设置** 中修改并自动保存。主要文件如下：

| 文件 | Linux 路径 | Windows 路径 |
| --- | --- | --- |
| 设置 | `~/.config/alter/settings.conf` | `%APPDATA%\Alter\settings.conf` |
| Quick Links | `~/.config/alter/quick-links.json` | `%APPDATA%\Alter\quick-links.json` |
| 网页搜索 | `~/.config/alter/web-searches.json` | `%APPDATA%\Alter\web-searches.json` |
| Workflows | `~/.config/alter/workflows/*.json` | `%APPDATA%\Alter\workflows\*.json` |
| Snippets | `~/.config/alter/snippets.json` | `%APPDATA%\Alter\snippets.json` |
| 历史与排序 | `~/.local/share/alter/history.sqlite3` | `%LOCALAPPDATA%\Alter\history.sqlite3` |

所有扩展命令都以 argv 参数数组启动，不经过 shell。请确认自定义命令和 URL 模板来自可信来源。

界面默认跟随系统语言，也可以在设置中手动选择 **English** 或 **简体中文**。

## 剪贴板说明

剪贴板默认保留 30 天，可在设置中调整为 1–3650 天。在 Linux 上，Alter 会读取 Clipse 的历史文件，但不会改写该文件。剪贴板内容保存在本机，可能包含密码或令牌等敏感文本，请按需清理或暂停剪贴板记录。

## 已知限制

- layer-shell 浮层和托盘集成仅用于 Linux/Wayland；Windows 使用无托盘的单次居中窗口。
- 自带剪贴板监听器只记录文本；Linux 上的图片和文件历史需要 Clipse。
- Wayland 不允许应用无条件向其他客户端注入 `Ctrl+V`，因此 Snippet 和剪贴板结果只会写入剪贴板。
- Linux 文件搜索结果取决于 `plocate` 索引是否及时更新。

## 许可证

MIT
