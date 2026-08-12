use crate::i18n::Language;
use gtk::gdk_pixbuf::Pixbuf;
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, Tray};
use std::path::Path;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Toggle,
    Settings,
    Quit,
}

struct AlterTray {
    sender: Sender<TrayAction>,
    icon: Vec<Icon>,
    language: Language,
}

impl Tray for AlterTray {
    fn id(&self) -> String {
        "alter".to_owned()
    }

    fn title(&self) -> String {
        "Alter".to_owned()
    }

    fn icon_name(&self) -> String {
        "alter".to_owned()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Alter".to_owned(),
            description: self
                .language
                .text(
                    "全局搜索、剪贴板和计算器",
                    "Global search, clipboard and calculator",
                )
                .to_owned(),
            icon_pixmap: self.icon.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayAction::Toggle);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayAction::Settings);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let open_sender = self.sender.clone();
        let settings_sender = self.sender.clone();
        let quit_sender = self.sender.clone();
        vec![
            StandardItem {
                label: self.language.text("打开 Alter", "Open Alter").to_owned(),
                icon_name: "system-search".to_owned(),
                activate: Box::new(move |_| {
                    let _ = open_sender.send(TrayAction::Toggle);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.language.text("打开设置", "Open Settings").to_owned(),
                icon_name: "preferences-system".to_owned(),
                activate: Box::new(move |_| {
                    let _ = settings_sender.send(TrayAction::Settings);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.language.text("退出 Alter", "Quit Alter").to_owned(),
                icon_name: "application-exit".to_owned(),
                activate: Box::new(move |_| {
                    let _ = quit_sender.send(TrayAction::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayHandle {
    handle: ksni::blocking::Handle<AlterTray>,
}

pub fn start(
    sender: Sender<TrayAction>,
    icon_path: Option<&Path>,
    language: Language,
) -> Result<TrayHandle, String> {
    let icon = icon_path.map(load_icon).unwrap_or_default();
    AlterTray {
        sender,
        icon,
        language,
    }
    .spawn()
    .map(|handle| TrayHandle { handle })
    .map_err(|error| format!("cannot start status notifier: {error:?}"))
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}

fn load_icon(path: &Path) -> Vec<Icon> {
    [16, 24, 32, 48, 64]
        .into_iter()
        .filter_map(|size| Pixbuf::from_file_at_scale(path, size, size, true).ok())
        .filter_map(pixbuf_to_icon)
        .collect()
}

fn pixbuf_to_icon(pixbuf: Pixbuf) -> Option<Icon> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = pixbuf.n_channels();
    let rowstride = pixbuf.rowstride();
    if width <= 0 || height <= 0 || rowstride <= 0 || !(channels == 3 || channels == 4) {
        return None;
    }
    let bytes = pixbuf.read_pixel_bytes();
    let bytes = bytes.as_ref();
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let row_start = (y * rowstride) as usize;
        for x in 0..width {
            let offset = row_start + (x * channels) as usize;
            let red = *bytes.get(offset)?;
            let green = *bytes.get(offset + 1)?;
            let blue = *bytes.get(offset + 2)?;
            let alpha = if channels == 4 {
                *bytes.get(offset + 3)?
            } else {
                255
            };
            // StatusNotifierItem expects ARGB32 in network byte order.
            data.extend([alpha, red, green, blue]);
        }
    }
    Some(Icon {
        width,
        height,
        data,
    })
}
