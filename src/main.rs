pub mod actions;
mod calculator;
mod clipboard;
pub mod clipboard_meta;
mod desktop;
mod i18n;
mod paths;
mod search;
mod settings;
pub mod snippets;
mod tray;
mod ui;
pub mod usage;
pub mod web;
pub mod workflow;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use search::SearchEngine;
use std::cell::{Cell, RefCell};
use std::env;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

const APPLICATION_ID: &str = "io.github.alter.Launcher";

/// Describes what a command-line client wants the primary instance to do.
///
/// `GApplication` forwards command-line invocations over D-Bus, so the
/// launcher and the clipboard shortcut can share the same daemon/window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchMode {
    Toggle,
    Clipboard,
    Daemon,
}

fn launch_mode_from_arguments(arguments: &[String]) -> LaunchMode {
    if arguments.iter().any(|argument| argument == "--clipboard") {
        LaunchMode::Clipboard
    } else if arguments.iter().any(|argument| argument == "--daemon") {
        LaunchMode::Daemon
    } else {
        LaunchMode::Toggle
    }
}

fn launch_mode_from_command_line(command_line: &gio::ApplicationCommandLine) -> LaunchMode {
    let options = command_line.options_dict();
    let option_enabled = |name: &str| options.lookup::<bool>(name).ok().flatten().unwrap_or(false);

    if option_enabled("clipboard") {
        LaunchMode::Clipboard
    } else if option_enabled("daemon") {
        LaunchMode::Daemon
    } else {
        // Keep this fallback for older GLib versions or launchers that pass
        // the flag as an unparsed argument.
        let arguments = command_line.arguments();
        if arguments
            .iter()
            .any(|argument| argument.to_string_lossy() == "--clipboard")
        {
            LaunchMode::Clipboard
        } else if arguments
            .iter()
            .any(|argument| argument.to_string_lossy() == "--daemon")
        {
            LaunchMode::Daemon
        } else {
            LaunchMode::Toggle
        }
    }
}

fn main() -> glib::ExitCode {
    let arguments: Vec<String> = env::args().collect();
    if arguments.iter().any(|argument| argument == "--capture") {
        let preferences = settings::load();
        return match clipboard::capture_stdin_with_retention(
            &clipboard::database_path(),
            preferences.clipboard_retention_days,
        ) {
            Ok(_) => glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("alter: cannot store clipboard: {error}");
                glib::ExitCode::FAILURE
            }
        };
    }

    if arguments
        .iter()
        .any(|argument| argument == "--version" || argument == "-V")
    {
        println!("alter 0.1.1");
        return glib::ExitCode::SUCCESS;
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print_help();
        return glib::ExitCode::SUCCESS;
    }
    let initial_mode = launch_mode_from_arguments(&arguments);

    let database = clipboard::database_path();
    if let Err(error) = clipboard::ensure_database(&database) {
        eprintln!("alter: clipboard history is unavailable: {error}");
    }
    if let Err(error) = usage::ensure_schema(&database) {
        eprintln!("alter: usage ranking is unavailable: {error}");
    }
    if let Err(error) = clipboard_meta::ensure_schema(&database) {
        eprintln!("alter: clipboard metadata is unavailable: {error}");
    }

    let apps = desktop::load_applications();
    let preferences = settings::shared(settings::load());
    let language = settings::snapshot(&preferences).language.effective();
    let engine = SearchEngine::new(apps, database.clone(), preferences.clone());
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("alter"));
    let state: Rc<RefCell<Option<ui::UiState>>> = Rc::new(RefCell::new(None));
    let pending_mode = Rc::new(Cell::new(initial_mode));
    let tray_handle: Rc<RefCell<Option<tray::TrayHandle>>> = Rc::new(RefCell::new(None));
    let application_hold = Rc::new(RefCell::new(None::<gio::ApplicationHoldGuard>));
    let (tray_sender, tray_receiver) = mpsc::channel::<tray::TrayAction>();

    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // Register the small set of options that are forwarded to the primary
    // GApplication instance.  Without these entries GLib would reject the
    // flags before the command-line signal reaches the daemon.
    for (name, description) in [
        ("toggle", "Toggle the Alter window"),
        ("clipboard", "Open directly in clipboard search"),
        ("daemon", "Start hidden and keep the tray resident"),
    ] {
        application.add_main_option(
            name,
            glib::Char::from(0u8),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            description,
            None,
        );
    }

    {
        let pending_mode = pending_mode.clone();
        application.connect_command_line(move |application, command_line| {
            let mode = launch_mode_from_command_line(command_line);
            let application = application.clone();
            let pending_mode = pending_mode.clone();
            // Presenting a GtkWindow while GLib is still servicing the
            // command-line D-Bus method can leave the Wayland surface mapped
            // only logically.  Activate on the next main-loop turn, after the
            // D-Bus reply has completed, so layer-shell can map it normally.
            glib::idle_add_local_once(move || {
                pending_mode.set(mode);
                application.activate();
            });
            glib::ExitCode::SUCCESS
        });
    }

    {
        let tray_handle = tray_handle.clone();
        let application_hold = application_hold.clone();
        application.connect_startup(move |application| {
            // A daemon launch intentionally creates its window hidden.
            // Startup runs only in the primary GApplication instance, so
            // command-line clients such as `alter --toggle` still exit after
            // forwarding their activation request.
            *application_hold.borrow_mut() = Some(application.hold());
            match tray::start(tray_sender.clone(), paths::icon_path().as_deref(), language) {
                Ok(handle) => *tray_handle.borrow_mut() = Some(handle),
                Err(error) => eprintln!("alter: tray icon is unavailable: {error}"),
            }
        });
    }

    {
        let state = state.clone();
        let engine = engine.clone();
        let database = database.clone();
        let executable = executable.clone();
        let pending_mode = pending_mode.clone();
        application.connect_activate(move |application| {
            let mode = pending_mode.replace(LaunchMode::Toggle);
            if let Some(existing) = state.borrow().as_ref() {
                match mode {
                    LaunchMode::Clipboard => existing.show_clipboard(),
                    LaunchMode::Daemon => existing.hide(),
                    LaunchMode::Toggle => {
                        if existing.window.is_visible() {
                            existing.hide();
                        } else {
                            existing.show();
                        }
                    }
                }
                return;
            }

            let watcher = match clipboard::ClipboardWatcher::start(&executable) {
                Ok(watcher) => watcher,
                Err(error) => {
                    eprintln!("alter: clipboard watcher is unavailable: {error}");
                    None
                }
            };
            let new_state = ui::build(
                application,
                engine.clone(),
                database.clone(),
                preferences.clone(),
                watcher,
            );
            match mode {
                LaunchMode::Clipboard => new_state.show_clipboard(),
                LaunchMode::Daemon => new_state.hide(),
                LaunchMode::Toggle => {}
            }
            *state.borrow_mut() = Some(new_state);
        });
    }

    {
        let application = application.clone();
        let state = state.clone();
        glib::timeout_add_local(Duration::from_millis(75), move || {
            while let Ok(action) = tray_receiver.try_recv() {
                match action {
                    tray::TrayAction::Toggle => application.activate(),
                    tray::TrayAction::Settings => {
                        if state.borrow().is_none() {
                            application.activate();
                        }
                        if let Some(state) = state.borrow().as_ref() {
                            state.show_settings();
                        }
                    }
                    tray::TrayAction::Quit => application.quit(),
                }
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let state = state.clone();
        let tray_handle = tray_handle.clone();
        application.connect_shutdown(move |_| {
            if let Some(mut state) = state.borrow_mut().take() {
                state.stop_watcher();
            }
            tray_handle.borrow_mut().take();
        });
    }

    // The application ID makes all shortcut invocations reach the existing
    // daemon.  Registered options are forwarded through GApplication's
    // command-line D-Bus method so `--clipboard` can select a dedicated scope.
    application.run_with_args(&arguments)
}

fn print_help() {
    println!(
        r#"Alter — Wayland launcher, global search and clipboard history

Usage:
  alter [--toggle] | [--clipboard] | [--daemon] | [--capture]

Keys:
  Enter       launch/open/copy selected result
  Up / Down   choose a result
  Tab / Right  open actions for an app or file
  Left / Backspace  return from actions
  Ctrl+Shift+P  pin/unpin the selected clipboard item
  Delete      hide the selected clipboard item
  Esc         hide Alter
  Ctrl+,      open settings

Scopes:
  a <query>   applications
  f <query>   files
  c <query>   clipboard history

Web:
  ? <query>    DuckDuckGo search
  web <query>  DuckDuckGo search
  g <query>    Google search
  b <query>    Bing search
  ddg <query>  DuckDuckGo search

Extensions:
  ~/.config/alter/workflows/*.json  Workflow manifests
  script_filter=true                Alfred-style JSON/TSV workflow results
  actions=[...]                     Named Workflow actions shown with Tab
  ~/.config/alter/snippets.json     Snippet definitions

Startup:
  --daemon    start hidden, keep the tray icon and wait for the shortcut
  --capture   read one text item from stdin into clipboard history
  --toggle    activate the existing daemon and toggle its window
  --clipboard activate the existing daemon in clipboard-only search mode"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn launch_arguments_select_dedicated_modes() {
        assert_eq!(
            launch_mode_from_arguments(&args(&["alter", "--toggle"])),
            LaunchMode::Toggle
        );
        assert_eq!(
            launch_mode_from_arguments(&args(&["alter", "--daemon"])),
            LaunchMode::Daemon
        );
        assert_eq!(
            launch_mode_from_arguments(&args(&["alter", "--clipboard"])),
            LaunchMode::Clipboard
        );
    }

    #[test]
    fn clipboard_mode_wins_over_background_when_both_are_present() {
        assert_eq!(
            launch_mode_from_arguments(&args(&["alter", "--daemon", "--clipboard"])),
            LaunchMode::Clipboard
        );
    }
}
