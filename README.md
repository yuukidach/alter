# Alter

[![AUR version](https://img.shields.io/aur/version/alter-launcher?label=AUR)](https://aur.archlinux.org/packages/alter-launcher) [![GitHub release](https://img.shields.io/github/v/release/yuukidach/alter?display_name=tag)](https://github.com/yuukidach/alter/releases) [![License](https://img.shields.io/github/license/yuukidach/alter)](LICENSE) [![简体中文](https://img.shields.io/badge/简体中文-README.zh--CN-blue)](README.zh-CN.md)

Alter is a fast, keyboard-first launcher for **Hyprland + Wayland**. It combines application and file search, clipboard history, web search, and lightweight extensions in one GTK4 overlay.

![Alter search interface](screenshots/alter-search.png)

## Features

- Launch `.desktop` applications with fuzzy search.
- Search local files with `plocate` or `fd` as a fallback.
- Browse clipboard history with optional [Clipse](https://github.com/savedra1/clipse) integration, including text, image, and file previews.
- Use the built-in calculator, web search, Quick Links, Workflows, and Snippets.
- Keep Alter in the Waybar tray with light/dark themes and English/Chinese UI.

## Requirements

Alter targets Linux systems running Hyprland and Wayland.

On Arch Linux or EndeavourOS:

```bash
sudo pacman -S --needed base-devel rust gtk4 gtk4-layer-shell \
  plocate fd wl-clipboard curl xdg-utils
```

`plocate`, `fd`, and Clipse are optional. Clipse is needed for image and file clipboard history; without it, Alter's built-in watcher stores text entries only.

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

| Path | Purpose |
| --- | --- |
| `~/.config/alter/settings.conf` | Feature switches, theme, language, retention period |
| `~/.config/alter/quick-links.json` | Keyword + URL shortcuts |
| `~/.config/alter/web-searches.json` | Custom web-search templates |
| `~/.config/alter/workflows/*.json` | User-defined commands and actions |
| `~/.config/alter/snippets.json` | Text templates using `{query}` |
| `~/.local/share/alter/history.sqlite3` | Clipboard metadata and usage ranking |

All extension commands are started as argument arrays, not through a shell. Keep custom commands and URL templates under your control.

The interface follows the system locale by default. Choose **English** or **Simplified Chinese** in Settings to override it.

## Clipboard notes

Alter keeps clipboard entries for 30 days by default, configurable from 1 to 3650 days. Clipse entries are read from Clipse's history file; Alter does not rewrite that file. Clipboard data is stored locally and may contain sensitive text.

## Limitations

- Hyprland/Wayland is the supported desktop environment; X11, GNOME, and KDE integrations are not currently provided.
- The built-in clipboard watcher records text only. Use Clipse for image and file history.
- Wayland prevents applications from injecting an unconditional `Ctrl+V` into another client; Snippets and clipboard results copy to the clipboard instead.
- File search quality depends on the freshness of the `plocate` index.

## License

MIT
