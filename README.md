# Alter

[![Website](https://img.shields.io/badge/Website-Visit-8b5cf6)](https://yuukidach.github.io/alter/) [![AUR version](https://img.shields.io/aur/version/alter-launcher?label=AUR)](https://aur.archlinux.org/packages/alter-launcher) [![GitHub release](https://img.shields.io/github/v/release/yuukidach/alter?display_name=tag)](https://github.com/yuukidach/alter/releases) [![License](https://img.shields.io/github/license/yuukidach/alter)](LICENSE) [![简体中文](https://img.shields.io/badge/简体中文-README.zh--CN-blue)](README.zh-CN.md)

Alter is a fast, keyboard-first launcher for **Linux/Wayland and Windows**. It combines application and file search, clipboard history, web search, and lightweight extensions in one GTK4 window.

![Alter search interface](screenshots/alter-search-rounded.png)

## Features

- Launch Linux `.desktop` applications or Windows Start Menu shortcuts with fuzzy search.
- Search local files with `plocate`, `fd`, or a built-in Windows fallback.
- Browse clipboard history with optional [Clipse](https://github.com/savedra1/clipse) integration, including text, image, and file previews.
- Use the built-in calculator, web search, Quick Links, Workflows, and Snippets.
- Use light/dark themes and English/Chinese UI; Linux also provides a Waybar-compatible tray icon.

## Requirements

### Linux

For the full overlay and tray integration, use Hyprland on Wayland.

On Arch Linux or EndeavourOS:

```bash
sudo pacman -S --needed base-devel rust gtk4 gtk4-layer-shell \
  plocate fd wl-clipboard curl xdg-utils
```

`plocate`, `fd`, and Clipse are optional. Clipse is needed for image and file clipboard history; without it, Alter's built-in watcher stores text entries only.

### Windows

Install [MSYS2](https://www.msys2.org/) and open its **UCRT64** terminal, then install the native Rust and GTK4 packages:

```bash
pacman -Syu
pacman -S --needed git mingw-w64-ucrt-x86_64-rust \
  mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-pkgconf
```

## Install

### AUR

```bash
yay -S alter-launcher
```

### From source

```bash
cargo build --release
./target/release/alter --daemon
```

On Windows, run these commands in the MSYS2 UCRT64 terminal:

```bash
cargo build --release
./target/release/alter.exe
```

Keep `C:\msys64\ucrt64\bin` on `PATH` when starting the executable, because it contains the GTK runtime DLLs. Windows uses a normal centered window and exits after the launcher is dismissed; bind `alter.exe` with your preferred Windows shortcut utility. `--daemon` and the tray icon are currently Linux-only.

## Hyprland setup

Start one background instance, then bind the shortcuts you want:

```ini
exec-once = /path/to/alter --daemon
bind = SUPER, SPACE, exec, /path/to/alter --toggle
bind = SUPER SHIFT, C, exec, /path/to/alter --clipboard
```

Reload Hyprland with `hyprctl reload`. The daemon stays hidden until a shortcut is pressed and reuses the same window for later invocations.

## Usage

| Input | Action |
| --- | --- |
| `a query` | Search applications |
| `f query` | Search files |
| `c query` | Search clipboard history |
| `? query` / `web query` | Google search |
| `g query` / `b query` / `ddg query` | Google / Bing / DuckDuckGo |
| `settings` or `Ctrl+,` | Open settings |

Useful keys:

- `Enter` — launch, open, or copy the selected result
- `↑` / `↓` — change selection
- `Tab` / `→` — open actions for an app, file, or Workflow result
- `←` / `Backspace` — return from the actions page
- `Ctrl+Shift+P` — pin or unpin a clipboard item
- `Delete` — hide a clipboard item in Alter
- `Esc` — hide the overlay

Command-line modes:

```text
alter --daemon       Start hidden and keep the tray resident
alter --toggle       Toggle the existing instance
alter --clipboard    Open directly in clipboard search
alter --capture      Read one text item from stdin into clipboard history
```

## Configuration

Most options are available from **Settings** and are saved automatically. The main files are:

| File | Linux location | Windows location |
| --- | --- | --- |
| Settings | `~/.config/alter/settings.conf` | `%APPDATA%\Alter\settings.conf` |
| Quick Links | `~/.config/alter/quick-links.json` | `%APPDATA%\Alter\quick-links.json` |
| Web searches | `~/.config/alter/web-searches.json` | `%APPDATA%\Alter\web-searches.json` |
| Workflows | `~/.config/alter/workflows/*.json` | `%APPDATA%\Alter\workflows\*.json` |
| Snippets | `~/.config/alter/snippets.json` | `%APPDATA%\Alter\snippets.json` |
| History and ranking | `~/.local/share/alter/history.sqlite3` | `%LOCALAPPDATA%\Alter\history.sqlite3` |

All extension commands are started as argument arrays, not through a shell. Keep custom commands and URL templates under your control.

The interface follows the system locale by default. Choose **English** or **Simplified Chinese** in Settings to override it.

## Clipboard notes

Alter keeps clipboard entries for 30 days by default, configurable from 1 to 3650 days. On Linux, Clipse entries are read from Clipse's history file; Alter does not rewrite that file. Clipboard data is stored locally and may contain sensitive text.

## Limitations

- The layer-shell overlay and tray integration are Linux/Wayland features; Windows uses a one-shot centered window without a tray icon.
- The built-in clipboard watcher records text only. On Linux, use Clipse for image and file history.
- Wayland prevents applications from injecting an unconditional `Ctrl+V` into another client; Snippets and clipboard results copy to the clipboard instead.
- On Linux, file search quality depends on the freshness of the `plocate` index.

## License

MIT
