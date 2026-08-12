use crate::actions::{
    self, ActionDescriptor, ActionKind, ActionTarget, TargetKind, TrashConfirmation,
};
use crate::clipboard::{self, ClipboardWatcher};
use crate::i18n::{self, Language, LanguagePreference};
use crate::search::{ResultKind, ResultPayload, SearchEngine, SearchResult};
use crate::settings::{self, Settings, SharedSettings, Theme};
use crate::workflow::WorkflowMatch;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, ApplicationWindow, Box as GtkBox, Button, CenterBox, ContentFit, DropDown, Image, Label,
    ListBox, ListBoxRow, Orientation, Paned, Picture, ScrolledWindow, SearchEntry, SelectionMode,
    SpinButton, Stack, Switch, TextView, WrapMode,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

struct SearchMessage {
    generation: u64,
    results: Vec<SearchResult>,
}

struct SettingsWidgets {
    page: GtkBox,
    done: Button,
    file_switch: Switch,
    clipboard_switch: Switch,
    web_switch: Switch,
    suggestions_switch: Switch,
    workflow_switch: Switch,
    snippet_switch: Switch,
    learning_switch: Switch,
    recent_switch: Switch,
    max_results: SpinButton,
    retention_days: SpinButton,
    theme: DropDown,
    language: DropDown,
    status: Label,
}

struct ActionWidgets {
    page: GtkBox,
    list: ListBox,
    title: Label,
    subtitle: Label,
    back: Button,
}

/// Widgets used by the right-hand preview pane on the search page.
///
/// The pane stays mounted while the result list changes.  This avoids
/// rebuilding a second widget tree for every key press and, more importantly,
/// lets the selected row update the preview immediately for both mouse and
/// keyboard navigation.
#[derive(Clone)]
struct PreviewWidgets {
    language: Language,
    panel: GtkBox,
    title: Label,
    subtitle: Label,
    stack: Stack,
    empty_title: Label,
    empty_subtitle: Label,
    text_view: TextView,
    image: Picture,
    content_meta: Label,
}

#[derive(Clone)]
enum ActionContext {
    File {
        target: ActionTarget,
        descriptors: Vec<ActionDescriptor>,
    },
    Workflow {
        matched: Box<WorkflowMatch>,
        arg: String,
    },
}

pub struct UiState {
    pub window: ApplicationWindow,
    entry: SearchEntry,
    stack: Stack,
    preview: PreviewWidgets,
    refresh: Rc<dyn Fn()>,
    watcher: Option<ClipboardWatcher>,
}

impl UiState {
    pub fn show(&self) {
        self.stack.set_visible_child_name("search");
        self.entry.set_text("");
        self.preview.panel.set_visible(false);
        update_preview(&self.preview, None);
        // `present()` alone does not reliably remap a layer-shell surface
        // after GTK hid it with `set_visible(false)` on all compositors.
        self.window.set_visible(true);
        self.window.present();
        // The compositor may have remapped a hidden layer surface.  Grabbing
        // focus after present keeps the first keystroke from going elsewhere.
        self.window.set_focus_visible(true);
        (self.refresh)();
    }

    /// Show the launcher with the clipboard scope already selected.
    ///
    /// The dedicated `Super+Shift+C` entry point intentionally keeps the
    /// normal launcher window and daemon, but starts with the same `c ` scope
    /// users can type manually in global search.
    pub fn show_clipboard(&self) {
        self.stack.set_visible_child_name("search");
        self.entry.set_text("c ");
        self.entry.set_position(-1);
        self.preview.panel.set_visible(true);
        update_preview(&self.preview, None);
        self.window.set_visible(true);
        self.window.present();
        self.window.set_focus_visible(true);
        (self.refresh)();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn show_settings(&self) {
        self.stack.set_visible_child_name("settings");
        self.window.set_visible(true);
        self.window.present();
    }

    pub fn stop_watcher(&mut self) {
        self.watcher.take();
    }
}

pub fn build(
    app: &gtk::Application,
    engine: SearchEngine,
    database: PathBuf,
    preferences: SharedSettings,
    watcher: Option<ClipboardWatcher>,
) -> UiState {
    install_css();
    let language = settings::snapshot(&preferences).language.effective();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Alter")
        .decorated(false)
        .resizable(false)
        .default_width(1000)
        .default_height(540)
        .build();

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("alter"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        // With no anchors on either axis, wlr-layer-shell centers the surface
        // on the selected output instead of pinning it to the top edge.
        window.set_anchor(Edge::Top, false);
        window.set_anchor(Edge::Bottom, false);
        window.set_anchor(Edge::Left, false);
        window.set_anchor(Edge::Right, false);
        window.set_exclusive_zone(0);
    } else {
        // This is only a development fallback; Alter is primarily a
        // Wayland/layer-shell application.
        window.set_modal(true);
    }

    let surface = GtkBox::new(Orientation::Vertical, 0);
    surface.add_css_class("surface");
    if settings::snapshot(&preferences).theme == Theme::Light {
        surface.add_css_class("light");
    }
    surface.set_width_request(1000);
    surface.set_height_request(540);

    let search_page = GtkBox::new(Orientation::Vertical, 0);
    search_page.add_css_class("search-page");
    search_page.set_vexpand(true);

    let header = GtkBox::new(Orientation::Horizontal, 12);
    header.add_css_class("search-header");
    header.set_margin_top(18);
    header.set_margin_start(20);
    header.set_margin_end(20);
    header.set_margin_bottom(10);

    let brand = GtkBox::new(Orientation::Horizontal, 0);
    brand.add_css_class("brand-block");
    let mark = brand_icon(40);
    mark.add_css_class("mark-icon");
    brand.append(&mark);
    brand.set_valign(Align::Center);
    header.append(&brand);

    let entry = SearchEntry::new();
    entry.set_placeholder_text(Some(language.text(
        "搜索应用、文件和剪贴板…",
        "Search apps, files and clipboard…",
    )));
    entry.set_hexpand(true);
    entry.set_valign(Align::Center);
    entry.add_css_class("search-entry");
    header.append(&entry);

    let settings_button = Button::new();
    let settings_image = Image::from_icon_name(settings_icon_name());
    settings_image.set_pixel_size(20);
    settings_button.set_child(Some(&settings_image));
    settings_button.set_tooltip_text(Some(language.text("设置（Ctrl+,）", "Settings (Ctrl+,)")));
    settings_button.add_css_class("icon-button");
    settings_button.set_focusable(false);
    settings_button.set_valign(Align::Center);
    header.append(&settings_button);
    search_page.append(&header);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.set_vexpand(true);
    list.add_css_class("results");
    list.set_margin_start(12);
    list.set_margin_end(12);

    let scroller = ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scroller.add_css_class("results-scroller");

    let preview = build_preview_panel(language);
    preview.panel.set_visible(false);
    let search_body = Paned::new(Orientation::Horizontal);
    search_body.add_css_class("search-body");
    search_body.set_vexpand(true);
    search_body.set_hexpand(true);
    search_body.set_position(640);
    search_body.set_resize_start_child(true);
    search_body.set_resize_end_child(false);
    search_body.set_shrink_start_child(false);
    // The preview contains labels and a text view whose natural width can be
    // larger than the pane we want to reserve. Allow it to shrink so the
    // clipboard list keeps the larger share of the launcher surface.
    search_body.set_shrink_end_child(true);
    search_body.set_wide_handle(false);
    search_body.set_start_child(Some(&scroller));
    search_body.set_end_child(Some(&preview.panel));
    search_page.append(&search_body);

    let footer = GtkBox::new(Orientation::Horizontal, 12);
    footer.add_css_class("search-footer");
    footer.set_margin_start(22);
    footer.set_margin_end(22);
    footer.set_margin_top(8);
    footer.set_margin_bottom(14);
    let status = Label::new(Some(language.text("正在准备索引…", "Preparing index…")));
    status.set_xalign(0.0);
    status.set_hexpand(true);
    status.set_valign(Align::Center);
    status.add_css_class("status");
    footer.append(&status);
    let shortcuts = Label::new(Some(language.text(
        "↑↓ 选择   Enter 打开   Tab / → 操作   c 范围预览   Esc 关闭",
        "↑↓ Select   Enter Open   Tab / → Actions   c Clipboard preview   Esc Close",
    )));
    shortcuts.add_css_class("shortcut-hint");
    shortcuts.set_xalign(1.0);
    shortcuts.set_valign(Align::Center);
    footer.append(&shortcuts);
    search_page.append(&footer);

    let settings_widgets = build_settings_page(&preferences, language);
    // Keep the launcher surface at its compact search height even when the
    // preferences page grows.  The settings content remains fully reachable
    // with a normal vertical scrollbar instead of forcing the layer surface
    // beyond the user's usable screen area.
    let settings_scroller = ScrolledWindow::builder()
        .child(&settings_widgets.page)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(false)
        .propagate_natural_width(false)
        .build();
    settings_scroller.add_css_class("settings-scroller");
    let action_widgets = build_action_page(language);
    let stack = Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&search_page, Some("search"));
    stack.add_named(&settings_scroller, Some("settings"));
    stack.add_named(&action_widgets.page, Some("actions"));
    stack.set_visible_child_name("search");
    surface.append(&stack);

    window.set_child(Some(&surface));

    let results = Rc::new(RefCell::new(Vec::<SearchResult>::new()));
    let action_context = Rc::new(RefCell::new(None::<ActionContext>));
    let generation = Rc::new(Cell::new(0u64));
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let (sender, receiver) = mpsc::channel::<SearchMessage>();
    let receiver = Rc::new(RefCell::new(receiver));

    let refresh: Rc<dyn Fn()> = {
        let entry = entry.clone();
        let sender = sender.clone();
        let engine = engine.clone();
        let generation = generation.clone();
        let pending = pending.clone();
        Rc::new(move || {
            let query = entry.text().to_string();
            request_search(
                query,
                sender.clone(),
                engine.clone(),
                generation.clone(),
                pending.clone(),
            );
            entry.grab_focus();
        })
    };

    {
        let stack = stack.clone();
        settings_button.connect_clicked(move |_| {
            stack.set_visible_child_name("settings");
        });
    }
    {
        let stack = stack.clone();
        let entry = entry.clone();
        settings_widgets.done.connect_clicked(move |_| {
            stack.set_visible_child_name("search");
            entry.grab_focus();
        });
    }
    {
        let stack = stack.clone();
        let entry = entry.clone();
        action_widgets.back.connect_clicked(move |_| {
            stack.set_visible_child_name("search");
            entry.grab_focus();
        });
    }
    {
        let action_context = action_context.clone();
        let window = window.clone();
        let stack = stack.clone();
        action_widgets.list.connect_row_activated(move |_, row| {
            activate_action(row.index(), &action_context, &window, &stack, language);
        });
    }
    wire_settings(
        &settings_widgets,
        &preferences,
        &surface,
        &refresh,
        language,
    );

    {
        let sender = sender.clone();
        let engine = engine.clone();
        let generation = generation.clone();
        let pending = pending.clone();
        let preview = preview.clone();
        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            set_preview_visibility(&preview, &query);
            request_search(
                query,
                sender.clone(),
                engine.clone(),
                generation.clone(),
                pending.clone(),
            );
        });
    }

    {
        let receiver = receiver.clone();
        let generation = generation.clone();
        let list = list.clone();
        let scroller = scroller.clone();
        let results = results.clone();
        let status = status.clone();
        let preview = preview.clone();
        glib::timeout_add_local(Duration::from_millis(45), move || {
            let mut newest = None;
            while let Ok(message) = receiver.borrow().try_recv() {
                newest = Some(message);
            }
            if let Some(message) = newest
                && message.generation == generation.get()
            {
                render_results(
                    &list,
                    &scroller,
                    &results,
                    &status,
                    &preview,
                    language,
                    message.results,
                );
            }
            glib::ControlFlow::Continue
        });
    }

    // Selection changes come from both pointer clicks and the keyboard
    // navigation handled below.  Keeping this as a ListBox signal means the
    // preview never lags behind the highlighted result.
    {
        let results = results.clone();
        let preview = preview.clone();
        list.connect_row_selected(move |_, row| {
            let selected = row.and_then(|row| results.borrow().get(row.index() as usize).cloned());
            update_preview(&preview, selected.as_ref());
        });
    }

    {
        let results = results.clone();
        let window = window.clone();
        let database = database.clone();
        let stack = stack.clone();
        let preferences = preferences.clone();
        list.connect_row_activated(move |list, row| {
            activate_index(
                row.index(),
                list,
                &results,
                &window,
                &database,
                &stack,
                &preferences,
            );
        });
    }

    {
        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let list_for_keys = list.clone();
        let scroller_for_keys = scroller.clone();
        let results_for_keys = results.clone();
        let window_for_keys = window.clone();
        let database_for_keys = database.clone();
        let preferences_for_keys = preferences.clone();
        let stack_for_keys = stack.clone();
        let entry_for_keys = entry.clone();
        let action_list_for_keys = action_widgets.list.clone();
        let action_title_for_keys = action_widgets.title.clone();
        let action_subtitle_for_keys = action_widgets.subtitle.clone();
        let action_context_for_keys = action_context.clone();
        let refresh_for_keys = refresh.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            use gdk::Key;
            let in_settings = stack_for_keys
                .visible_child_name()
                .is_some_and(|name| name.as_str() == "settings");
            let in_actions = stack_for_keys
                .visible_child_name()
                .is_some_and(|name| name.as_str() == "actions");
            if key == Key::comma && modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                stack_for_keys.set_visible_child_name("settings");
                return glib::Propagation::Stop;
            }
            if key == Key::Escape {
                if in_settings || in_actions {
                    stack_for_keys.set_visible_child_name("search");
                    entry_for_keys.grab_focus();
                } else {
                    window_for_keys.set_visible(false);
                }
                return glib::Propagation::Stop;
            }
            if in_settings {
                return glib::Propagation::Proceed;
            }
            if in_actions {
                if key == Key::Return || key == Key::KP_Enter {
                    if let Some(row) = action_list_for_keys.selected_row() {
                        activate_action(
                            row.index(),
                            &action_context_for_keys,
                            &window_for_keys,
                            &stack_for_keys,
                            language,
                        );
                    }
                    return glib::Propagation::Stop;
                }
                if key == Key::Left || key == Key::BackSpace {
                    stack_for_keys.set_visible_child_name("search");
                    entry_for_keys.grab_focus();
                    return glib::Propagation::Stop;
                }
                let step = if key == Key::Down {
                    Some(1)
                } else if key == Key::Up {
                    Some(-1)
                } else {
                    None
                };
                if let Some(step) = step {
                    move_selection_without_scroller(&action_list_for_keys, step);
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }
            if key == Key::P
                && modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                if let Some(row) = list_for_keys.selected_row()
                    && let Some(result) = results_for_keys.borrow().get(row.index() as usize)
                    && toggle_clipboard_pin(result, &database_for_keys)
                {
                    refresh_for_keys();
                }
                return glib::Propagation::Stop;
            }
            if key == Key::Delete {
                if let Some(row) = list_for_keys.selected_row()
                    && let Some(result) = results_for_keys.borrow().get(row.index() as usize)
                    && hide_clipboard_result(result, &database_for_keys)
                {
                    refresh_for_keys();
                    return glib::Propagation::Stop;
                }
                // Let the focused SearchEntry handle Delete for ordinary text
                // queries.  Clipboard hiding only consumes the key when a
                // clipboard row was actually selected.
                return glib::Propagation::Proceed;
            }
            if (key == Key::Tab || key == Key::Right)
                && let Some(row) = list_for_keys.selected_row()
                && let Some(result) = results_for_keys.borrow().get(row.index() as usize)
                && show_action_panel(
                    result,
                    &action_list_for_keys,
                    &action_title_for_keys,
                    &action_subtitle_for_keys,
                    &action_context_for_keys,
                    &stack_for_keys,
                    language,
                )
            {
                return glib::Propagation::Stop;
            }
            if key == Key::Return || key == Key::KP_Enter {
                if let Some(row) = list_for_keys.selected_row() {
                    activate_index(
                        row.index(),
                        &list_for_keys,
                        &results_for_keys,
                        &window_for_keys,
                        &database_for_keys,
                        &stack_for_keys,
                        &preferences_for_keys,
                    );
                }
                return glib::Propagation::Stop;
            }

            let step = if key == Key::Down
                || (key == Key::N && modifiers.contains(gdk::ModifierType::CONTROL_MASK))
            {
                Some(1)
            } else if key == Key::Up
                || (key == Key::P && modifiers.contains(gdk::ModifierType::CONTROL_MASK))
            {
                Some(-1)
            } else {
                None
            };
            if let Some(step) = step {
                move_selection(&list_for_keys, &scroller_for_keys, step);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);
    }

    {
        let window = window.clone();
        window.connect_close_request(move |window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });
    }

    window.present();
    entry.grab_focus();
    (refresh)();

    UiState {
        window,
        entry,
        stack,
        preview,
        refresh,
        watcher,
    }
}

fn set_preview_visibility(preview: &PreviewWidgets, query: &str) {
    let should_show = is_clipboard_scope_query(query);
    if should_show != preview.panel.is_visible() {
        preview.panel.set_visible(should_show);
        // Do not briefly show the previous app/file result while the
        // clipboard search is being recomputed on the worker thread.
        update_preview(preview, None);
    }
}

fn is_clipboard_scope_query(query: &str) -> bool {
    let Some((prefix, _)) = query.split_once(char::is_whitespace) else {
        return false;
    };
    matches!(
        prefix.to_ascii_lowercase().as_str(),
        "c" | "clip" | "clipboard"
    )
}

fn request_search(
    query: String,
    sender: mpsc::Sender<SearchMessage>,
    engine: SearchEngine,
    generation: Rc<Cell<u64>>,
    pending: Rc<RefCell<Option<glib::SourceId>>>,
) {
    let next_generation = generation.get().wrapping_add(1);
    generation.set(next_generation);
    if let Some(source) = pending.borrow_mut().take() {
        source.remove();
    }

    let pending_for_callback = pending.clone();
    let source = glib::timeout_add_local_once(Duration::from_millis(70), move || {
        pending_for_callback.borrow_mut().take();
        std::thread::spawn(move || {
            let results = engine.search(&query);
            let _ = sender.send(SearchMessage {
                generation: next_generation,
                results,
            });
        });
    });
    *pending.borrow_mut() = Some(source);
}

fn render_results(
    list: &ListBox,
    scroller: &ScrolledWindow,
    results_cell: &Rc<RefCell<Vec<SearchResult>>>,
    status: &Label,
    preview: &PreviewWidgets,
    language: Language,
    new_results: Vec<SearchResult>,
) {
    *results_cell.borrow_mut() = new_results;
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    {
        let results = results_cell.borrow();
        for result in results.iter() {
            let display_result = i18n::localized_result(result, language);
            let row = ListBoxRow::new();
            row.add_css_class(match result.kind {
                ResultKind::App => "app-row",
                ResultKind::File => "file-row",
                ResultKind::Clipboard => "clipboard-row",
                ResultKind::Calculation => "calculation-row",
                ResultKind::Settings => "settings-row",
                ResultKind::Web => "web-row",
                ResultKind::Workflow => "workflow-row",
                ResultKind::Snippet => "snippet-row",
            });
            if result.clipboard_pinned {
                row.add_css_class("pinned-row");
            }

            let content = GtkBox::new(Orientation::Horizontal, 12);
            content.add_css_class("result-content");
            content.set_valign(Align::Center);
            content.set_margin_top(8);
            content.set_margin_bottom(8);
            content.set_margin_start(10);
            content.set_margin_end(10);

            let icon_frame = CenterBox::new();
            icon_frame.add_css_class("result-icon-frame");
            icon_frame.add_css_class(match result.kind {
                ResultKind::App => "app-icon-frame",
                ResultKind::File => "file-icon-frame",
                ResultKind::Clipboard => "clipboard-icon-frame",
                ResultKind::Calculation => "calculation-icon-frame",
                ResultKind::Settings => "settings-icon-frame",
                ResultKind::Web => "web-icon-frame",
                ResultKind::Workflow => "workflow-icon-frame",
                ResultKind::Snippet => "snippet-icon-frame",
            });
            icon_frame.set_valign(Align::Center);
            icon_frame.set_halign(Align::Center);
            icon_frame.set_size_request(46, 46);
            icon_frame.set_center_widget(Some(&result_icon(result)));
            content.append(&icon_frame);

            let badge = Label::new(Some(i18n::result_kind_label(&result.kind, language)));
            badge.add_css_class("result-kind");
            badge.add_css_class(match result.kind {
                ResultKind::App => "app-badge",
                ResultKind::File => "file-badge",
                ResultKind::Clipboard => "clipboard-badge",
                ResultKind::Calculation => "calculation-badge",
                ResultKind::Settings => "settings-badge",
                ResultKind::Web => "web-badge",
                ResultKind::Workflow => "workflow-badge",
                ResultKind::Snippet => "snippet-badge",
            });
            badge.set_valign(Align::Center);
            badge.set_xalign(0.5);
            badge.set_width_request(58);

            let labels = GtkBox::new(Orientation::Vertical, 2);
            labels.add_css_class("result-labels");
            labels.set_hexpand(true);
            labels.set_valign(Align::Center);
            let title = Label::new(Some(&display_result.title));
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.add_css_class("result-title");
            labels.append(&title);
            let subtitle = Label::new(Some(&display_result.subtitle));
            subtitle.set_xalign(0.0);
            subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
            subtitle.add_css_class("result-subtitle");
            labels.append(&subtitle);
            content.append(&labels);
            let pin_slot = GtkBox::new(Orientation::Horizontal, 0);
            pin_slot.add_css_class("pin-slot");
            pin_slot.set_valign(Align::Center);
            pin_slot.set_halign(Align::Center);
            if result.clipboard_pinned {
                let pin = Label::new(Some("★"));
                pin.add_css_class("pin-indicator");
                pin.set_valign(Align::Center);
                pin_slot.append(&pin);
            }
            content.append(&pin_slot);
            content.append(&badge);
            let arrow = themed_icon(&["go-next-symbolic"], "go-next-symbolic");
            arrow.set_pixel_size(16);
            arrow.add_css_class("result-arrow");
            arrow.set_valign(Align::Center);
            content.append(&arrow);
            row.set_child(Some(&content));
            list.append(&row);
        }
    }

    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    // A new result set always starts at its first row. Without resetting the
    // adjustment, a query entered after scrolling could leave that row above
    // the visible viewport.
    let adjustment = scroller.vadjustment();
    adjustment.set_value(adjustment.lower());

    let result_count = results_cell.borrow().len();
    if result_count == 0 {
        status.set_text(language.text(
            "未找到结果 · 可输入算式，或用 a / f / c 限定范围",
            "No results · Try an expression, or use a / f / c to limit the scope",
        ));
    } else {
        let status_text = if language == Language::Chinese {
            format!(
                "{} 个结果  ·  a 应用  ·  f 文件  ·  c 剪贴板  ·  ? 网页",
                result_count
            )
        } else {
            format!(
                "{} results  ·  a Apps  ·  f Files  ·  c Clipboard  ·  ? Web",
                result_count
            )
        };
        status.set_text(&status_text);
    }

    let selected = list
        .selected_row()
        .and_then(|row| results_cell.borrow().get(row.index() as usize).cloned());
    update_preview(preview, selected.as_ref());
}

fn build_preview_panel(language: Language) -> PreviewWidgets {
    let panel = GtkBox::new(Orientation::Vertical, 0);
    panel.add_css_class("preview-panel");
    panel.set_width_request(330);
    panel.set_hexpand(false);
    panel.set_vexpand(true);

    let header = GtkBox::new(Orientation::Vertical, 4);
    header.add_css_class("preview-header");
    header.set_margin_top(16);
    header.set_margin_start(18);
    header.set_margin_end(18);
    header.set_margin_bottom(12);

    let kicker = Label::new(Some(language.text("剪贴板预览", "Clipboard preview")));
    kicker.add_css_class("preview-kicker");
    kicker.set_xalign(0.0);
    header.append(&kicker);

    let title = Label::new(Some(
        language.text("选择一条剪贴板记录", "Select a clipboard item"),
    ));
    title.add_css_class("preview-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_single_line_mode(true);
    header.append(&title);

    let subtitle = Label::new(Some(
        language.text("使用 ↑ / ↓ 浏览记录", "Use ↑ / ↓ to browse items"),
    ));
    subtitle.add_css_class("preview-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.set_single_line_mode(true);
    header.append(&subtitle);
    panel.append(&header);

    let stack = Stack::new();
    stack.add_css_class("preview-content-stack");
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    let empty = GtkBox::new(Orientation::Vertical, 8);
    empty.add_css_class("preview-empty");
    empty.set_valign(Align::Center);
    empty.set_halign(Align::Center);
    empty.set_margin_start(28);
    empty.set_margin_end(28);

    let empty_icon = themed_icon(&["edit-paste-symbolic"], "edit-paste-symbolic");
    empty_icon.set_pixel_size(30);
    empty_icon.add_css_class("preview-empty-icon");
    empty_icon.set_halign(Align::Center);
    empty.append(&empty_icon);

    let empty_title = Label::new(Some(
        language.text("选择一条剪贴板记录", "Select a clipboard item"),
    ));
    empty_title.add_css_class("preview-empty-title");
    empty_title.set_xalign(0.5);
    empty_title.set_wrap(true);
    empty.append(&empty_title);

    let empty_subtitle = Label::new(Some(language.text(
        "使用 ↑ / ↓ 浏览，右侧会显示完整内容",
        "Use ↑ / ↓ to browse; the full content appears here",
    )));
    empty_subtitle.add_css_class("preview-empty-subtitle");
    empty_subtitle.set_xalign(0.5);
    empty_subtitle.set_wrap(true);
    empty.append(&empty_subtitle);
    stack.add_named(&empty, Some("empty"));

    let text_view = TextView::new();
    text_view.add_css_class("preview-text");
    text_view.set_editable(false);
    text_view.set_cursor_visible(false);
    text_view.set_focusable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(WrapMode::WordChar);
    text_view.set_top_margin(14);
    text_view.set_bottom_margin(14);
    text_view.set_left_margin(14);
    text_view.set_right_margin(14);
    text_view.set_vexpand(true);
    text_view.set_hexpand(true);
    let text_scroller = ScrolledWindow::builder()
        .child(&text_view)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    text_scroller.add_css_class("preview-text-scroller");
    stack.add_named(&text_scroller, Some("text"));

    let image = Picture::new();
    image.add_css_class("preview-image");
    image.set_can_shrink(true);
    image.set_content_fit(ContentFit::Contain);
    image.set_vexpand(true);
    image.set_hexpand(true);
    image.set_halign(Align::Center);
    image.set_valign(Align::Center);
    let image_scroller = ScrolledWindow::builder()
        .child(&image)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    image_scroller.add_css_class("preview-image-scroller");
    stack.add_named(&image_scroller, Some("image"));
    stack.set_visible_child_name("empty");
    panel.append(&stack);

    let content_meta = Label::new(Some(language.text(
        "选择一条剪贴板记录以查看完整内容",
        "Select a clipboard item to view its full content",
    )));
    content_meta.add_css_class("preview-meta");
    content_meta.set_xalign(0.0);
    content_meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content_meta.set_margin_start(18);
    content_meta.set_margin_end(18);
    content_meta.set_margin_top(10);
    content_meta.set_margin_bottom(14);
    panel.append(&content_meta);

    PreviewWidgets {
        language,
        panel,
        title,
        subtitle,
        stack,
        empty_title,
        empty_subtitle,
        text_view,
        image,
        content_meta,
    }
}

fn update_preview(preview: &PreviewWidgets, result: Option<&SearchResult>) {
    let language = preview.language;
    let Some(result) = result else {
        preview
            .title
            .set_text(language.text("选择一条剪贴板记录", "Select a clipboard item"));
        preview
            .subtitle
            .set_text(language.text("使用 ↑ / ↓ 浏览记录", "Use ↑ / ↓ to browse items"));
        preview
            .empty_title
            .set_text(language.text("选择一条剪贴板记录", "Select a clipboard item"));
        preview.empty_subtitle.set_text(language.text(
            "使用 ↑ / ↓ 浏览，右侧会显示完整内容",
            "Use ↑ / ↓ to browse; the full content appears here",
        ));
        preview.content_meta.set_text(language.text(
            "选择一条剪贴板记录以查看完整内容",
            "Select a clipboard item to view its full content",
        ));
        preview.stack.set_visible_child_name("empty");
        return;
    };

    if result.kind != ResultKind::Clipboard {
        preview
            .title
            .set_text(language.text("剪贴板预览", "Clipboard preview"));
        preview
            .subtitle
            .set_text(&i18n::localized_result(result, language).title);
        preview.empty_title.set_text(language.text(
            "此结果没有剪贴板内容",
            "This result has no clipboard content",
        ));
        preview.empty_subtitle.set_text(language.text(
            "选中剪贴板结果后将在这里显示完整预览",
            "Select a clipboard result to see its full preview",
        ));
        preview.content_meta.set_text("");
        preview.stack.set_visible_child_name("empty");
        return;
    }

    preview
        .title
        .set_text(language.text("剪贴板预览", "Clipboard preview"));
    preview.subtitle.set_text(&result.title);

    let Some(content) = result.clipboard_content.as_deref() else {
        preview.empty_title.set_text(language.text(
            "该记录没有可预览的内容",
            "This item has no previewable content",
        ));
        preview.empty_subtitle.set_text(language.text(
            "按 Enter 仍可尝试写回剪贴板",
            "Press Enter to try writing it back to the clipboard",
        ));
        let localized = i18n::localized_result(result, language);
        preview.content_meta.set_text(&localized.subtitle);
        preview.stack.set_visible_child_name("empty");
        return;
    };

    if let Some(path) = result
        .clipboard_path
        .as_deref()
        .filter(|path| path.is_file() && is_image_path(path))
    {
        preview.image.set_filename(Some(path));
        preview.image.set_alternative_text(Some(&result.title));
        preview.stack.set_visible_child_name("image");
        let meta = if language == Language::Chinese {
            format!("图片 · {} · Enter 写回剪贴板", path.to_string_lossy())
        } else {
            format!("Image · {} · Enter to write back", path.to_string_lossy())
        };
        preview.content_meta.set_text(&meta);
        return;
    }

    preview.text_view.buffer().set_text(content);
    preview.stack.set_visible_child_name("text");
    let character_count = content.chars().count();
    let byte_count = content.len();
    if let Some(path) = result.clipboard_path.as_deref() {
        let meta = if language == Language::Chinese {
            format!(
                "文件 · {} · {character_count} 字符 · {byte_count} 字节 · Enter 写回剪贴板",
                path.to_string_lossy()
            )
        } else {
            format!(
                "File · {} · {character_count} chars · {byte_count} bytes · Enter to write back",
                path.to_string_lossy()
            )
        };
        preview.content_meta.set_text(&meta);
    } else {
        let meta = if language == Language::Chinese {
            format!("{character_count} 字符 · {byte_count} 字节 · Enter 写回剪贴板")
        } else {
            format!("{character_count} chars · {byte_count} bytes · Enter to write back")
        };
        preview.content_meta.set_text(&meta);
    }
}

fn build_action_page(language: Language) -> ActionWidgets {
    let page = GtkBox::new(Orientation::Vertical, 0);
    page.add_css_class("action-page");
    page.set_vexpand(true);

    let header = GtkBox::new(Orientation::Horizontal, 12);
    header.add_css_class("action-header");
    header.set_margin_top(18);
    header.set_margin_start(20);
    header.set_margin_end(20);
    header.set_margin_bottom(12);

    let back = Button::new();
    let back_icon = themed_icon(&["go-previous-symbolic"], "go-previous-symbolic");
    back_icon.set_pixel_size(20);
    back_icon.add_css_class("back-icon");
    back.set_child(Some(&back_icon));
    back.add_css_class("back-button");
    back.set_tooltip_text(Some(
        language.text("返回搜索（← / Esc）", "Back to search (← / Esc)"),
    ));
    header.append(&back);

    let labels = GtkBox::new(Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = Label::new(Some(language.text("可用操作", "Available actions")));
    title.add_css_class("action-title");
    title.set_xalign(0.0);
    labels.append(&title);
    let subtitle = Label::new(Some(
        language.text("对选中的结果执行操作", "Actions for the selected result"),
    ));
    subtitle.add_css_class("action-subtitle");
    subtitle.set_xalign(0.0);
    labels.append(&subtitle);
    header.append(&labels);
    page.append(&header);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.set_vexpand(true);
    list.add_css_class("action-list");
    list.set_margin_start(18);
    list.set_margin_end(18);
    let scroller = ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    page.append(&scroller);

    let hint = Label::new(Some(language.text(
        "↑↓ 选择   Enter 执行   ← 返回",
        "↑↓ Select   Enter Run   ← Back",
    )));
    hint.add_css_class("action-hint");
    hint.set_xalign(0.0);
    hint.set_margin_start(22);
    hint.set_margin_top(10);
    hint.set_margin_bottom(16);
    page.append(&hint);

    ActionWidgets {
        page,
        list,
        title,
        subtitle,
        back,
    }
}

fn show_action_panel(
    result: &SearchResult,
    list: &ListBox,
    title: &Label,
    subtitle: &Label,
    context: &Rc<RefCell<Option<ActionContext>>>,
    stack: &Stack,
    language: Language,
) -> bool {
    title.set_text(language.text("结果操作", "Result actions"));
    subtitle.set_text(&result.title);
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    match result.kind {
        ResultKind::App | ResultKind::File => {
            let target = if result.kind == ResultKind::App {
                ActionTarget::application(result.target.clone())
            } else {
                ActionTarget::from_path(result.target.clone())
            };
            let descriptors = actions::actions_for_target(&target);
            for descriptor in &descriptors {
                let (action_title, action_subtitle) =
                    action_text(descriptor.kind, target.kind(), language);
                append_action_row(
                    list,
                    action_title,
                    action_subtitle,
                    themed_icon(
                        &[action_icon_name(descriptor.kind)],
                        action_icon_name(descriptor.kind),
                    ),
                    descriptor.destructive,
                );
            }
            *context.borrow_mut() = Some(ActionContext::File {
                target,
                descriptors,
            });
        }
        ResultKind::Workflow => {
            let (matched, arg) = match result.payload.as_ref() {
                Some(ResultPayload::Workflow(matched)) => (matched.clone(), matched.query.clone()),
                Some(ResultPayload::WorkflowItem { matched, item }) => {
                    let arg = if item.arg.is_empty() {
                        matched.query.clone()
                    } else {
                        item.arg.clone()
                    };
                    (matched.clone(), arg)
                }
                _ => return false,
            };
            if matched.workflow.actions.is_empty() {
                return false;
            }
            for action in &matched.workflow.actions {
                let icon = action
                    .icon
                    .as_deref()
                    .and_then(|icon| application_icon(icon, &matched.workflow.source))
                    .unwrap_or_else(|| {
                        themed_icon(&["system-run-symbolic"], "system-run-symbolic")
                    });
                let action_subtitle = if action.subtitle.is_empty() {
                    language.text("执行 Workflow 动作", "Run Workflow action")
                } else {
                    &action.subtitle
                };
                append_action_row(list, &action.title, action_subtitle, icon, false);
            }
            *context.borrow_mut() = Some(ActionContext::Workflow {
                matched: Box::new(matched),
                arg,
            });
        }
        _ => return false,
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    stack.set_visible_child_name("actions");
    true
}

fn append_action_row(list: &ListBox, title: &str, subtitle: &str, icon: Image, destructive: bool) {
    let row = ListBoxRow::new();
    row.add_css_class("action-row");
    if destructive {
        row.add_css_class("destructive-action");
    }
    let content = GtkBox::new(Orientation::Horizontal, 12);
    content.set_valign(Align::Center);
    content.set_margin_top(9);
    content.set_margin_bottom(9);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let icon_frame = CenterBox::new();
    icon_frame.add_css_class("action-icon-frame");
    icon_frame.set_valign(Align::Center);
    icon_frame.set_halign(Align::Center);
    icon_frame.set_size_request(40, 40);
    icon.set_pixel_size(22);
    icon.set_size_request(22, 22);
    icon.set_valign(Align::Center);
    icon.add_css_class("action-icon");
    icon_frame.set_center_widget(Some(&icon));
    content.append(&icon_frame);

    let labels = GtkBox::new(Orientation::Vertical, 2);
    labels.set_hexpand(true);
    labels.set_valign(Align::Center);
    let action_title = Label::new(Some(title));
    action_title.add_css_class("action-row-title");
    action_title.set_xalign(0.0);
    labels.append(&action_title);
    let action_subtitle = Label::new(Some(subtitle));
    action_subtitle.add_css_class("action-row-subtitle");
    action_subtitle.set_xalign(0.0);
    labels.append(&action_subtitle);
    content.append(&labels);

    let arrow = themed_icon(&["go-next-symbolic"], "go-next-symbolic");
    arrow.set_pixel_size(16);
    arrow.add_css_class("result-arrow");
    arrow.set_valign(Align::Center);
    content.append(&arrow);
    row.set_child(Some(&content));
    list.append(&row);
}

fn action_text(
    kind: ActionKind,
    target_kind: TargetKind,
    language: Language,
) -> (&'static str, &'static str) {
    if language == Language::Chinese {
        return match (kind, target_kind) {
            (ActionKind::Open, TargetKind::File) => ("打开文件", "使用默认应用打开"),
            (ActionKind::Open, TargetKind::Directory) => ("打开目录", "使用文件管理器打开"),
            (ActionKind::Open, TargetKind::Application) => ("启动应用", "运行所选应用程序"),
            (ActionKind::Reveal, TargetKind::File) => ("打开所在目录", "使用文件管理器打开"),
            (ActionKind::Reveal, TargetKind::Directory) => ("打开上级目录", "使用文件管理器打开"),
            (ActionKind::Reveal, TargetKind::Application) => {
                ("打开应用条目所在目录", "使用文件管理器打开")
            }
            (ActionKind::CopyPath, _) => ("复制路径", "将完整路径复制到剪贴板"),
            (ActionKind::CopyUri, _) => ("复制文件 URI", "复制经过安全转义的 file:// URI"),
            (ActionKind::MoveToTrash, _) => ("移入回收站", "执行前需要再次确认"),
        };
    }
    match (kind, target_kind) {
        (ActionKind::Open, TargetKind::File) => ("Open file", "Open with the default app"),
        (ActionKind::Open, TargetKind::Directory) => ("Open folder", "Open with the file manager"),
        (ActionKind::Open, TargetKind::Application) => ("Launch app", "Run the selected app"),
        (ActionKind::Reveal, TargetKind::File) => {
            ("Open containing folder", "Open with the file manager")
        }
        (ActionKind::Reveal, TargetKind::Directory) => {
            ("Open parent folder", "Open with the file manager")
        }
        (ActionKind::Reveal, TargetKind::Application) => {
            ("Open app's folder", "Open with the file manager")
        }
        (ActionKind::CopyPath, _) => ("Copy path", "Copy the full path to the clipboard"),
        (ActionKind::CopyUri, _) => ("Copy file URI", "Copy a safely escaped file:// URI"),
        (ActionKind::MoveToTrash, _) => ("Move to Trash", "Requires confirmation"),
    }
}

fn action_icon_name(kind: ActionKind) -> &'static str {
    let candidates: &[&'static str] = match kind {
        ActionKind::Open => &["document-open-symbolic", "document-open", "folder-symbolic"],
        ActionKind::Reveal => &[
            "folder-open-symbolic",
            "folder-open",
            "system-file-manager",
            "folder-symbolic",
        ],
        ActionKind::CopyPath => &["edit-copy-symbolic", "edit-copy", "copy"],
        ActionKind::CopyUri => &[
            "insert-link-symbolic",
            "insert-link",
            "edit-copy-symbolic",
            "edit-copy",
            "copy",
        ],
        ActionKind::MoveToTrash => &[
            "user-trash-symbolic",
            "user-trash",
            "edit-delete-symbolic",
            "edit-delete",
        ],
    };
    available_icon(candidates, "system-run-symbolic")
}

fn activate_action(
    index: i32,
    context: &Rc<RefCell<Option<ActionContext>>>,
    window: &ApplicationWindow,
    stack: &Stack,
    language: Language,
) {
    let Some(current) = context.borrow().clone() else {
        return;
    };
    match current {
        ActionContext::File {
            target,
            descriptors,
        } => {
            let Some(descriptor) = descriptors.get(index as usize).copied() else {
                return;
            };
            if descriptor.requires_confirmation {
                let parent = window.clone();
                let callback_window = window.clone();
                let (message, detail, cancel, confirm) = if language == Language::Chinese {
                    (
                        "移入回收站？",
                        format!(
                            "{}\n此操作会把项目移入系统回收站。",
                            target.path().display()
                        ),
                        "取消",
                        "移入回收站",
                    )
                } else {
                    (
                        "Move to Trash?",
                        format!(
                            "{}\nThis will move the item to the system Trash.",
                            target.path().display()
                        ),
                        "Cancel",
                        "Move to Trash",
                    )
                };
                let dialog = gtk::AlertDialog::builder()
                    .message(message)
                    .detail(detail)
                    .buttons([cancel, confirm])
                    .cancel_button(0)
                    .default_button(1)
                    .build();
                dialog.choose(
                    Some(&parent),
                    None::<&gtk::gio::Cancellable>,
                    move |answer| {
                        if matches!(answer, Ok(1)) {
                            std::thread::spawn(move || {
                                if let Err(error) = actions::execute_with_trash_confirmation(
                                    descriptor.kind,
                                    &target,
                                    TrashConfirmation::confirmed_by_user(),
                                ) {
                                    eprintln!("alter: file action failed: {error}");
                                }
                            });
                            callback_window.set_visible(false);
                        }
                    },
                );
                return;
            }

            let action = descriptor.kind;
            std::thread::spawn(move || {
                if let Err(error) = actions::execute(action, &target) {
                    eprintln!("alter: file action failed: {error}");
                }
            });
        }
        ActionContext::Workflow { matched, arg } => {
            if let Err(error) =
                matched
                    .workflow
                    .execute_action(index as usize, &matched.query, &arg)
            {
                eprintln!("alter: workflow action failed: {error}");
            }
        }
    }
    stack.set_visible_child_name("search");
    window.set_visible(false);
}

fn move_selection_without_scroller(list: &ListBox, step: i32) {
    let count = (0..)
        .take_while(|index| list.row_at_index(*index).is_some())
        .count() as i32;
    if count == 0 {
        return;
    }
    let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
    let next = (current + step).rem_euclid(count);
    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
    }
}

fn result_icon(result: &SearchResult) -> Image {
    let image = match result.kind {
        ResultKind::App => result
            .icon
            .as_deref()
            .and_then(|icon| application_icon(icon, &result.target))
            .unwrap_or_else(|| {
                bundled_icon("app-default.svg").unwrap_or_else(|| {
                    themed_icon(
                        &["view-app-grid-symbolic", "applications-utilities-symbolic"],
                        "applications-utilities-symbolic",
                    )
                })
            }),
        ResultKind::File if result.target.is_dir() => {
            themed_icon(&["folder-symbolic"], "folder-symbolic")
        }
        ResultKind::File => themed_icon(&["text-x-generic-symbolic"], "text-x-generic-symbolic"),
        ResultKind::Clipboard => clipboard_result_icon(result),
        ResultKind::Calculation => themed_icon(
            &["accessories-calculator-symbolic"],
            "accessories-calculator-symbolic",
        ),
        ResultKind::Settings => bundled_icon("settings.svg").unwrap_or_else(|| {
            themed_icon(
                &["preferences-system-symbolic"],
                "preferences-system-symbolic",
            )
        }),
        ResultKind::Web => themed_icon(&["web-browser-symbolic"], "web-browser-symbolic"),
        ResultKind::Workflow => result
            .icon
            .as_deref()
            .and_then(|icon| application_icon(icon, &result.target))
            .unwrap_or_else(|| {
                themed_icon(
                    &["applications-utilities-symbolic"],
                    "applications-utilities-symbolic",
                )
            }),
        ResultKind::Snippet => themed_icon(&["insert-text-symbolic"], "text-x-generic-symbolic"),
    };
    image.set_pixel_size(32);
    image.set_size_request(32, 32);
    image.set_valign(Align::Center);
    image.set_halign(Align::Center);
    image.add_css_class("result-icon");
    image
}

fn clipboard_result_icon(result: &SearchResult) -> Image {
    let Some(path) = result.clipboard_path.as_deref() else {
        return themed_icon(&["edit-paste-symbolic"], "edit-paste-symbolic");
    };
    if !path.is_file() {
        return if path.is_dir() {
            themed_icon(&["folder-symbolic"], "folder-symbolic")
        } else {
            themed_icon(&["text-x-generic-symbolic"], "text-x-generic-symbolic")
        };
    }
    if !is_image_path(path) {
        return themed_icon(&["text-x-generic-symbolic"], "text-x-generic-symbolic");
    }
    let image = Image::from_file(path);
    image.add_css_class("clipboard-thumbnail");
    image
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "jpe"
                    | "webp"
                    | "gif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "svg"
                    | "svgz"
                    | "avif"
            )
        })
}

fn brand_icon(pixel_size: i32) -> Image {
    let path = crate::paths::icon_path();
    let image = path
        .filter(|path| path.is_file())
        .map(Image::from_file)
        .unwrap_or_else(|| {
            themed_icon(
                &["view-app-grid-symbolic", "system-run-symbolic"],
                "system-run-symbolic",
            )
        });
    image.set_pixel_size(pixel_size);
    image.set_size_request(pixel_size, pixel_size);
    image.set_valign(Align::Center);
    image
}

fn settings_icon_name() -> &'static str {
    // Icon names vary a little between Papirus, Adwaita and minimal Wayland
    // sessions. Pick the first name the active theme actually provides so the
    // header never falls back to a distracting missing-icon warning glyph.
    const CANDIDATES: &[&str] = &[
        "preferences-system-symbolic",
        "preferences-system",
        "preferences",
        "emblem-system-symbolic",
        "settings",
    ];
    available_icon(CANDIDATES, "preferences")
}

fn available_icon(candidates: &[&'static str], fallback: &'static str) -> &'static str {
    let Some(display) = gdk::Display::default() else {
        return fallback;
    };
    let theme = gtk::IconTheme::for_display(&display);
    candidates
        .iter()
        .copied()
        .find(|name| theme.has_icon(name))
        .unwrap_or(fallback)
}

fn themed_icon(candidates: &[&'static str], fallback: &'static str) -> Image {
    Image::from_icon_name(available_icon(candidates, fallback))
}

fn bundled_icon(name: &str) -> Option<Image> {
    let bytes: &[u8] = match name {
        "app-default.svg" => include_bytes!("../resources/icons/app-default.svg"),
        "settings.svg" => include_bytes!("../resources/icons/settings.svg"),
        _ => return None,
    };
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);
    Some(Image::from_paintable(Some(&texture)))
}

fn application_icon(icon: &str, desktop_file: &Path) -> Option<Image> {
    let path = Path::new(icon);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        desktop_file.parent()?.join(path)
    };
    if candidate.is_file() {
        return Some(Image::from_file(candidate));
    }

    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    theme.has_icon(icon).then(|| Image::from_icon_name(icon))
}

fn move_selection(list: &ListBox, scroller: &ScrolledWindow, step: i32) {
    let count = (0..)
        .take_while(|index| list.row_at_index(*index).is_some())
        .count() as i32;
    if count == 0 {
        return;
    }
    let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
    let next = (current + step).rem_euclid(count);
    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
        scroll_row_into_view(list, &row, scroller);
    }
}

fn scroll_row_into_view(list: &ListBox, row: &ListBoxRow, scroller: &ScrolledWindow) {
    let adjustment = scroller.vadjustment();
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let row_top = f64::from(bounds.y());
    let row_bottom = row_top + f64::from(bounds.height());
    let viewport_top = adjustment.value();
    let viewport_bottom = viewport_top + adjustment.page_size();

    let target = if row_top < viewport_top {
        row_top
    } else if row_bottom > viewport_bottom {
        row_bottom - adjustment.page_size()
    } else {
        return;
    };
    let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(target.clamp(adjustment.lower(), maximum));
}

fn activate_index(
    index: i32,
    _list: &ListBox,
    results: &Rc<RefCell<Vec<SearchResult>>>,
    window: &ApplicationWindow,
    database: &Path,
    stack: &Stack,
    preferences: &SharedSettings,
) {
    let Some(result) = results.borrow().get(index as usize).cloned() else {
        return;
    };
    if settings::snapshot(preferences).learning_ranking {
        record_result_usage(database, &result);
    }

    match result.kind {
        ResultKind::App => {
            window.set_visible(false);
            let _ = Command::new("gio")
                .arg("launch")
                .arg(&result.target)
                .spawn();
        }
        ResultKind::File => {
            window.set_visible(false);
            let _ = Command::new("gio").arg("open").arg(&result.target).spawn();
        }
        ResultKind::Clipboard => {
            window.set_visible(false);
            let id = result.clipboard_id;
            let path = result.clipboard_path;
            let content = result.clipboard_content;
            if let Some(id) = id {
                let database = database.to_path_buf();
                std::thread::spawn(move || {
                    let _ = clipboard::mark_used(&database, id);
                    let copy_result = if let Some(path) = path {
                        clipboard::copy_file_to_wayland(&path)
                    } else if let Some(content) = content {
                        clipboard::copy_to_wayland(&content)
                    } else {
                        Ok(())
                    };
                    if let Err(error) = copy_result {
                        eprintln!("alter: clipboard copy failed: {error}");
                    }
                });
            }
        }
        ResultKind::Calculation => {
            window.set_visible(false);
            if let Some(content) = result.clipboard_content {
                std::thread::spawn(move || {
                    let _ = clipboard::copy_to_wayland(&content);
                });
            }
        }
        ResultKind::Settings => {
            stack.set_visible_child_name("settings");
        }
        ResultKind::Web => {
            if let Some(ResultPayload::Web(action)) = result.payload {
                window.set_visible(false);
                let _ = action.open();
            }
        }
        ResultKind::Workflow => match result.payload {
            Some(ResultPayload::Workflow(matched)) => {
                window.set_visible(false);
                if let Err(error) = matched
                    .workflow
                    .execute_result(&matched.query, &matched.query)
                {
                    eprintln!("alter: workflow failed: {error}");
                }
            }
            Some(ResultPayload::WorkflowItem { matched, item }) => {
                window.set_visible(false);
                let original_query = matched.query.clone();
                let query = if item.arg.is_empty() {
                    original_query.clone()
                } else {
                    item.arg
                };
                if let Err(error) = matched.workflow.execute_result(&original_query, &query) {
                    eprintln!("alter: workflow result failed: {error}");
                }
            }
            _ => {}
        },
        ResultKind::Snippet => {
            if let Some(ResultPayload::Snippet { content }) = result.payload {
                window.set_visible(false);
                std::thread::spawn(move || {
                    let _ = clipboard::copy_to_wayland(&content);
                });
            }
        }
    }
}

fn record_result_usage(database: &Path, result: &SearchResult) {
    let (key, kind) = match result.kind {
        ResultKind::App => (format!("app:{}", result.target.display()), "app"),
        ResultKind::File => (format!("file:{}", result.target.display()), "file"),
        ResultKind::Clipboard => (
            format!("clip:{}", result.clipboard_id.unwrap_or_default()),
            "clipboard",
        ),
        ResultKind::Calculation => (format!("calc:{}", result.title), "calculation"),
        ResultKind::Settings => ("settings".to_owned(), "settings"),
        ResultKind::Web => match result.payload.as_ref() {
            Some(ResultPayload::Web(action)) => (
                format!("web:{}:{}", action.provider_id, action.query),
                "web",
            ),
            _ => return,
        },
        ResultKind::Workflow => match result.payload.as_ref() {
            Some(ResultPayload::Workflow(matched)) => {
                (format!("workflow:{}", matched.workflow.id), "workflow")
            }
            Some(ResultPayload::WorkflowItem { matched, .. }) => {
                (format!("workflow:{}", matched.workflow.id), "workflow")
            }
            _ => return,
        },
        ResultKind::Snippet => (format!("snippet:{}", result.target.display()), "snippet"),
    };
    let _ = crate::usage::record_use(database, &key, &result.title, kind);
}

fn toggle_clipboard_pin(result: &SearchResult, database: &Path) -> bool {
    if result.kind != ResultKind::Clipboard {
        return false;
    }
    let Some(key) = clipboard_metadata_key(result) else {
        return false;
    };
    crate::clipboard_meta::set_pinned(database, &key, !result.clipboard_pinned).is_ok()
}

fn hide_clipboard_result(result: &SearchResult, database: &Path) -> bool {
    if result.kind != ResultKind::Clipboard {
        return false;
    }
    let Some(key) = clipboard_metadata_key(result) else {
        return false;
    };
    crate::clipboard_meta::set_hidden(database, &key, true).is_ok()
}

fn clipboard_metadata_key(result: &SearchResult) -> Option<String> {
    if result.kind != ResultKind::Clipboard {
        return None;
    }
    result
        .clipboard_path
        .as_deref()
        .map(crate::clipboard_meta::file_key)
        .or_else(|| {
            result
                .clipboard_content
                .as_deref()
                .map(crate::clipboard_meta::content_key)
        })
}

fn build_settings_page(preferences: &SharedSettings, language: Language) -> SettingsWidgets {
    let current = settings::snapshot(preferences);
    let page = GtkBox::new(Orientation::Vertical, 0);
    page.add_css_class("settings-page");
    page.set_vexpand(true);

    let header = GtkBox::new(Orientation::Horizontal, 10);
    header.add_css_class("settings-header");
    let settings_mark = brand_icon(30);
    settings_mark.add_css_class("settings-mark");
    settings_mark.set_valign(Align::Center);
    header.append(&settings_mark);
    let title = Label::new(Some(language.text("Alter 设置", "Alter Settings")));
    title.add_css_class("settings-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_valign(Align::Center);
    header.append(&title);
    let done = Button::with_label(language.text("完成", "Done"));
    done.add_css_class("done-button");
    done.set_valign(Align::Center);
    header.append(&done);
    page.append(&header);

    let description = Label::new(Some(language.text(
        "这些偏好会自动保存到 ~/.config/alter/settings.conf",
        "These preferences are saved automatically to ~/.config/alter/settings.conf",
    )));
    description.add_css_class("settings-description");
    description.set_xalign(0.0);
    description.set_wrap(true);
    page.append(&description);

    let search_group = GtkBox::new(Orientation::Vertical, 0);
    search_group.add_css_class("settings-group");

    let (file_row, file_switch) = switch_setting_row(
        language.text("文件搜索", "File search"),
        language.text(
            "使用 plocate / fd 搜索本机文件",
            "Search local files with plocate / fd",
        ),
        current.file_search,
    );
    search_group.append(&file_row);

    let (clipboard_row, clipboard_switch) = switch_setting_row(
        language.text("显示剪贴板结果", "Show clipboard results"),
        language.text(
            "在全局搜索中包含 Clipse 或 Alter 的历史",
            "Include Clipse or Alter history in global search",
        ),
        current.clipboard_search,
    );
    let clipboard_group = GtkBox::new(Orientation::Vertical, 0);
    clipboard_group.add_css_class("settings-group");
    clipboard_group.append(&clipboard_row);

    let web_group = GtkBox::new(Orientation::Vertical, 0);
    web_group.add_css_class("settings-group");
    let (web_row, web_switch) = switch_setting_row(
        language.text("网页搜索", "Web search"),
        language.text(
            "支持 g / b / ddg / ? 前缀，在浏览器中搜索",
            "Search in a browser with the g / b / ddg / ? prefixes",
        ),
        current.web_search,
    );
    web_group.append(&web_row);
    let (suggestions_row, suggestions_switch) = switch_setting_row(
        language.text("搜索建议", "Search suggestions"),
        language.text(
            "输入网页关键词时显示在线建议（网络失败会自动跳过）",
            "Show online suggestions for web queries (network errors are ignored)",
        ),
        current.web_suggestions,
    );
    web_group.append(&suggestions_row);

    let extensions_group = GtkBox::new(Orientation::Vertical, 0);
    extensions_group.add_css_class("settings-group");
    let (workflow_row, workflow_switch) = switch_setting_row(
        language.text("Workflow 搜索", "Workflow search"),
        language.text(
            "启用 ~/.config/alter/workflows 中的 Alfred 风格工作流",
            "Enable Alfred-style workflows in ~/.config/alter/workflows",
        ),
        current.workflow_search,
    );
    extensions_group.append(&workflow_row);
    let (snippet_row, snippet_switch) = switch_setting_row(
        language.text("Snippets 搜索", "Snippet search"),
        language.text(
            "启用 ~/.config/alter/snippets 中的可复制文本片段",
            "Enable copyable text snippets in ~/.config/alter/snippets",
        ),
        current.snippet_search,
    );
    extensions_group.append(&snippet_row);

    let ranking_group = GtkBox::new(Orientation::Vertical, 0);
    ranking_group.add_css_class("settings-group");
    let (learning_row, learning_switch) = switch_setting_row(
        language.text("学习排序", "Usage-based ranking"),
        language.text(
            "根据使用次数和最近使用时间调整结果顺序（数据仅保存在本机）",
            "Rank results by usage and recency (data stays on this device)",
        ),
        current.learning_ranking,
    );
    ranking_group.append(&learning_row);

    let (recent_row, recent_switch) = switch_setting_row(
        language.text(
            "空白搜索显示最近项目",
            "Show recent items for an empty query",
        ),
        language.text(
            "不输入关键词时显示最近应用和剪贴板",
            "Show recent apps and clipboard items without a query",
        ),
        current.show_recent,
    );
    search_group.append(&recent_row);

    let max_results = SpinButton::with_range(10.0, 100.0, 5.0);
    max_results.set_value(current.max_results as f64);
    max_results.set_numeric(true);
    max_results.set_digits(0);
    max_results.set_width_request(92);
    let max_row = control_setting_row(
        language.text("最多结果数", "Maximum results"),
        language.text(
            "搜索结果超过此数量时继续滚动查看",
            "Scroll to see more results beyond this number",
        ),
        &max_results,
    );
    search_group.append(&max_row);

    let retention_days = SpinButton::with_range(1.0, 3650.0, 1.0);
    retention_days.set_value(current.clipboard_retention_days as f64);
    retention_days.set_numeric(true);
    retention_days.set_digits(0);
    retention_days.set_width_request(92);
    let retention_row = control_setting_row(
        language.text("剪贴板保留天数", "Clipboard retention (days)"),
        language.text(
            "超过期限的本地和 Clipse 文本历史会自动清理（默认 30 天）",
            "Local and Clipse text history older than this is removed (30 days by default)",
        ),
        &retention_days,
    );
    clipboard_group.append(&retention_row);

    let theme = DropDown::from_strings(&[
        language.text("暗色主题", "Dark theme"),
        language.text("亮色主题", "Light theme"),
    ]);
    theme.set_selected(match current.theme {
        Theme::Dark => 0,
        Theme::Light => 1,
    });
    theme.set_width_request(128);
    let theme_row = control_setting_row(
        language.text("外观", "Appearance"),
        language.text("切换 Alter 浮层的配色", "Change Alter's overlay colors"),
        &theme,
    );
    let language_dropdown =
        DropDown::from_strings(&["跟随系统 / System", "简体中文 / Chinese", "English"]);
    language_dropdown.set_selected(current.language.selected_index());
    language_dropdown.set_width_request(180);
    let language_row = control_setting_row(
        language.text("界面语言", "Interface language"),
        language.text(
            "切换后重启 Alter 生效",
            "Restart Alter after changing the language",
        ),
        &language_dropdown,
    );
    let appearance_group = GtkBox::new(Orientation::Vertical, 0);
    appearance_group.add_css_class("settings-group");
    appearance_group.append(&theme_row);

    page.append(&settings_section_title(language.text("搜索", "Search")));
    page.append(&search_group);
    page.append(&settings_section_title(language.text("网页", "Web")));
    page.append(&web_group);
    page.append(&settings_section_title(language.text("扩展", "Extensions")));
    page.append(&extensions_group);
    page.append(&settings_section_title(
        language.text("剪贴板", "Clipboard"),
    ));
    page.append(&clipboard_group);
    page.append(&settings_section_title(
        language.text("隐私与排序", "Privacy & ranking"),
    ));
    page.append(&ranking_group);
    page.append(&settings_section_title(language.text("外观", "Appearance")));
    page.append(&appearance_group);
    page.append(&language_row);

    let status = Label::new(Some(
        language.text("修改会自动保存", "Changes are saved automatically"),
    ));
    status.add_css_class("settings-status");
    status.set_xalign(0.0);
    status.set_margin_top(12);
    page.append(&status);

    let shortcut = Label::new(Some(language.text(
        "快捷键：Super+Space 全局搜索 · Super+Shift+C 剪贴板 · Ctrl+, 设置 · Esc 返回/关闭",
        "Shortcuts: Super+Space Search · Super+Shift+C Clipboard · Ctrl+, Settings · Esc Back/close",
    )));
    shortcut.add_css_class("settings-shortcut");
    shortcut.set_xalign(0.0);
    shortcut.set_wrap(true);
    page.append(&shortcut);

    SettingsWidgets {
        page,
        done,
        file_switch,
        clipboard_switch,
        web_switch,
        suggestions_switch,
        workflow_switch,
        snippet_switch,
        learning_switch,
        recent_switch,
        max_results,
        retention_days,
        theme,
        language: language_dropdown,
        status,
    }
}

fn settings_section_title(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("settings-section-title");
    label.set_xalign(0.0);
    label
}

fn switch_setting_row(title: &str, description: &str, active: bool) -> (GtkBox, Switch) {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    row.add_css_class("setting-row");
    let text = setting_text(title, description);
    row.append(&text);
    let control = Switch::new();
    control.set_active(active);
    control.set_valign(Align::Center);
    control.add_css_class("setting-switch");
    row.append(&setting_control_slot(&control));
    (row, control)
}

fn control_setting_row<W: IsA<gtk::Widget>>(title: &str, description: &str, control: &W) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    row.add_css_class("setting-row");
    let text = setting_text(title, description);
    row.append(&text);
    control.set_valign(Align::Center);
    row.append(&setting_control_slot(control));
    row
}

fn setting_control_slot<W: IsA<gtk::Widget>>(control: &W) -> GtkBox {
    let slot = GtkBox::new(Orientation::Horizontal, 0);
    slot.add_css_class("setting-control-slot");
    slot.set_halign(Align::End);
    slot.set_valign(Align::Center);
    control.set_halign(Align::End);
    slot.append(control);
    slot
}

fn setting_text(title: &str, description: &str) -> GtkBox {
    let text = GtkBox::new(Orientation::Vertical, 3);
    text.set_hexpand(true);
    text.set_valign(Align::Center);
    let title_label = Label::new(Some(title));
    title_label.add_css_class("setting-title");
    title_label.set_xalign(0.0);
    text.append(&title_label);
    let description_label = Label::new(Some(description));
    description_label.add_css_class("setting-description");
    description_label.set_xalign(0.0);
    description_label.set_wrap(true);
    text.append(&description_label);
    text
}

fn wire_settings(
    widgets: &SettingsWidgets,
    preferences: &SharedSettings,
    surface: &GtkBox,
    refresh: &Rc<dyn Fn()>,
    language: Language,
) {
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.file_switch.connect_active_notify(move |switch| {
            save_setting(&preferences, &status, language, |settings| {
                settings.file_search = switch.is_active();
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.retention_days.connect_value_changed(move |spin| {
            save_setting(&preferences, &status, language, |settings| {
                settings.clipboard_retention_days = spin.value_as_int().max(1) as u32;
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets
            .clipboard_switch
            .connect_active_notify(move |switch| {
                save_setting(&preferences, &status, language, |settings| {
                    settings.clipboard_search = switch.is_active();
                });
                refresh();
            });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.web_switch.connect_active_notify(move |switch| {
            save_setting(&preferences, &status, language, |settings| {
                settings.web_search = switch.is_active();
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets
            .suggestions_switch
            .connect_active_notify(move |switch| {
                save_setting(&preferences, &status, language, |settings| {
                    settings.web_suggestions = switch.is_active();
                });
                refresh();
            });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets
            .workflow_switch
            .connect_active_notify(move |switch| {
                save_setting(&preferences, &status, language, |settings| {
                    settings.workflow_search = switch.is_active();
                });
                refresh();
            });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.snippet_switch.connect_active_notify(move |switch| {
            save_setting(&preferences, &status, language, |settings| {
                settings.snippet_search = switch.is_active();
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets
            .learning_switch
            .connect_active_notify(move |switch| {
                save_setting(&preferences, &status, language, |settings| {
                    settings.learning_ranking = switch.is_active();
                });
                refresh();
            });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.recent_switch.connect_active_notify(move |switch| {
            save_setting(&preferences, &status, language, |settings| {
                settings.show_recent = switch.is_active();
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let refresh = refresh.clone();
        widgets.max_results.connect_value_changed(move |spin| {
            save_setting(&preferences, &status, language, |settings| {
                settings.max_results = spin.value_as_int().max(10) as usize;
            });
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        let surface = surface.clone();
        let refresh = refresh.clone();
        widgets.theme.connect_selected_notify(move |dropdown| {
            let theme = if dropdown.selected() == 1 {
                Theme::Light
            } else {
                Theme::Dark
            };
            save_setting(&preferences, &status, language, |settings| {
                settings.theme = theme;
            });
            if theme == Theme::Light {
                surface.add_css_class("light");
            } else {
                surface.remove_css_class("light");
            }
            refresh();
        });
    }
    {
        let preferences = preferences.clone();
        let status = widgets.status.clone();
        widgets.language.connect_selected_notify(move |dropdown| {
            let preference = LanguagePreference::from_selected_index(dropdown.selected());
            save_setting(&preferences, &status, language, |settings| {
                settings.language = preference;
            });
            status.set_text(language.text(
                "已保存，重启 Alter 后生效",
                "Saved; restart Alter to apply the language",
            ));
        });
    }
}

fn save_setting<F>(preferences: &SharedSettings, status: &Label, language: Language, update: F)
where
    F: FnOnce(&mut Settings),
{
    let current = {
        let Ok(mut settings) = preferences.write() else {
            status.set_text(language.text("设置保存失败", "Failed to save settings"));
            return;
        };
        update(&mut settings);
        settings.clone()
    };
    match settings::save(&current) {
        Ok(()) => status.set_text(language.text("已保存", "Saved")),
        Err(error) => {
            eprintln!("alter: cannot save settings: {error}");
            status.set_text(language.text("设置保存失败", "Failed to save settings"));
        }
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../resources/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::is_clipboard_scope_query;

    #[test]
    fn preview_scope_requires_an_explicit_clipboard_prefix() {
        for query in ["c ", "c hello", "clip hello", "CLIPBOARD image"] {
            assert!(is_clipboard_scope_query(query), "expected scope: {query}");
        }
        for query in ["", "c", "clipboard", "hello", " c hello"] {
            assert!(
                !is_clipboard_scope_query(query),
                "unexpected scope: {query}"
            );
        }
    }
}
