//! Platforms without a StatusNotifierItem implementation still get the full
//! launcher. The ordinary application window owns the process lifetime.

use crate::i18n::Language;
use std::path::Path;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum TrayAction {
    Toggle,
    Settings,
    Quit,
}

pub struct TrayHandle;

pub fn start(
    _sender: Sender<TrayAction>,
    _icon_path: Option<&Path>,
    _language: Language,
) -> Result<TrayHandle, String> {
    Ok(TrayHandle)
}
