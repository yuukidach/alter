//! Reusable actions for file and application search results.
//!
//! The module intentionally contains no GTK code.  A UI can render the
//! descriptors returned by [`actions_for_target`] and execute the selected
//! action on a worker thread.  Moving an item to the trash is deliberately
//! unavailable through [`execute`]; callers must first ask the user and then
//! use [`execute_with_trash_confirmation`] with an explicit confirmation
//! token.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Open,
    Reveal,
    CopyPath,
    CopyUri,
    MoveToTrash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    File,
    Directory,
    Application,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionTarget {
    kind: TargetKind,
    path: PathBuf,
}

impl ActionTarget {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: TargetKind::File,
            path: path.into(),
        }
    }

    pub fn directory(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: TargetKind::Directory,
            path: path.into(),
        }
    }

    pub fn application(desktop_file: impl Into<PathBuf>) -> Self {
        Self {
            kind: TargetKind::Application,
            path: desktop_file.into(),
        }
    }

    /// Infer file versus directory from the current filesystem state.
    ///
    /// Search results normally point to existing paths.  A path that cannot
    /// be inspected is treated as a file; execution will still report a clear
    /// `TargetNotFound` error before attempting to open or trash it.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if path.is_dir() {
            Self::directory(path)
        } else {
            Self::file(path)
        }
    }

    pub fn kind(&self) -> TargetKind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionDescriptor {
    pub kind: ActionKind,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub destructive: bool,
    pub requires_confirmation: bool,
}

/// Return actions in the order that an action panel should normally show.
pub fn actions_for_target(target: &ActionTarget) -> Vec<ActionDescriptor> {
    let (open_title, open_subtitle, reveal_title) = match target.kind {
        TargetKind::File => ("打开文件", "使用默认应用打开", "打开所在目录"),
        TargetKind::Directory => ("打开目录", "使用文件管理器打开", "打开上级目录"),
        TargetKind::Application => ("启动应用", "运行所选应用程序", "打开应用条目所在目录"),
    };

    let mut actions = vec![
        descriptor(ActionKind::Open, open_title, open_subtitle, false, false),
        descriptor(
            ActionKind::Reveal,
            reveal_title,
            "使用文件管理器打开",
            false,
            false,
        ),
        descriptor(
            ActionKind::CopyPath,
            "复制路径",
            "将完整路径复制到剪贴板",
            false,
            false,
        ),
        descriptor(
            ActionKind::CopyUri,
            "复制文件 URI",
            "复制经过安全转义的 file:// URI",
            false,
            false,
        ),
    ];

    // Removing a system or Flatpak desktop entry is surprising and can break
    // application discovery.  Applications therefore expose launch/reveal/
    // copy actions, but never a trash action.
    if target.kind != TargetKind::Application {
        actions.push(descriptor(
            ActionKind::MoveToTrash,
            "移入回收站",
            "执行前需要再次确认",
            true,
            true,
        ));
    }
    actions
}

const fn descriptor(
    kind: ActionKind,
    title: &'static str,
    subtitle: &'static str,
    destructive: bool,
    requires_confirmation: bool,
) -> ActionDescriptor {
    ActionDescriptor {
        kind,
        title,
        subtitle,
        destructive,
        requires_confirmation,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    Opened,
    CopiedToClipboard,
    MovedToTrash,
}

#[derive(Debug)]
pub enum ActionError {
    EmptyTarget,
    TargetNotFound(PathBuf),
    Unsupported {
        action: ActionKind,
        target: TargetKind,
    },
    ConfirmationRequired,
    Io {
        operation: &'static str,
        source: io::Error,
    },
    CommandFailed {
        program: &'static str,
        status: Option<i32>,
    },
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTarget => formatter.write_str("the action target is empty"),
            Self::TargetNotFound(path) => {
                write!(
                    formatter,
                    "the action target does not exist: {}",
                    path.display()
                )
            }
            Self::Unsupported { action, target } => {
                write!(
                    formatter,
                    "action {action:?} is not supported for {target:?}"
                )
            }
            Self::ConfirmationRequired => {
                formatter.write_str("moving an item to the trash requires user confirmation")
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::CommandFailed { program, status } => match status {
                Some(code) => write!(formatter, "{program} exited with status {code}"),
                None => write!(formatter, "{program} was terminated by a signal"),
            },
        }
    }
}

impl Error for ActionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// An explicit capability required to move a target to the trash.
///
/// Construct this value only after the UI has shown the exact target path and
/// the user has confirmed the operation.
#[derive(Debug)]
pub struct TrashConfirmation(());

impl TrashConfirmation {
    pub fn confirmed_by_user() -> Self {
        Self(())
    }
}

/// Execute a non-destructive action.
///
/// `MoveToTrash` always returns `ConfirmationRequired`, even when the action
/// originated from a descriptor.  Use `execute_with_trash_confirmation` only
/// after displaying a confirmation dialog.
pub fn execute(action: ActionKind, target: &ActionTarget) -> Result<ActionOutcome, ActionError> {
    if action == ActionKind::MoveToTrash {
        if target.kind == TargetKind::Application {
            return Err(ActionError::Unsupported {
                action,
                target: target.kind,
            });
        }
        return Err(ActionError::ConfirmationRequired);
    }
    execute_inner(action, target)
}

/// Execute an action after the caller has explicitly confirmed a trash move.
///
/// Passing a confirmation token does not make application desktop entries
/// removable; that target/action pair remains unsupported by design.
pub fn execute_with_trash_confirmation(
    action: ActionKind,
    target: &ActionTarget,
    _confirmation: TrashConfirmation,
) -> Result<ActionOutcome, ActionError> {
    execute_inner(action, target)
}

fn execute_inner(action: ActionKind, target: &ActionTarget) -> Result<ActionOutcome, ActionError> {
    if action == ActionKind::MoveToTrash && target.kind == TargetKind::Application {
        return Err(ActionError::Unsupported {
            action,
            target: target.kind,
        });
    }
    validate_target(target, action)?;

    match action {
        ActionKind::Open => {
            if target.kind == TargetKind::Application {
                crate::platform::launch_application(target.path()).map_err(|source| {
                    ActionError::Io {
                        operation: "cannot launch application",
                        source,
                    }
                })?;
            } else {
                crate::platform::open_path(target.path()).map_err(|source| ActionError::Io {
                    operation: "cannot open target",
                    source,
                })?;
            }
            Ok(ActionOutcome::Opened)
        }
        ActionKind::Reveal => {
            let absolute = absolute_path(target.path())?;
            crate::platform::reveal_path(&absolute).map_err(|source| ActionError::Io {
                operation: "cannot reveal target",
                source,
            })?;
            Ok(ActionOutcome::Opened)
        }
        ActionKind::CopyPath | ActionKind::CopyUri => {
            let text = copy_text_for(action, target)?;
            crate::clipboard::copy_text(&text).map_err(|source| ActionError::Io {
                operation: "cannot copy to clipboard",
                source,
            })?;
            Ok(ActionOutcome::CopiedToClipboard)
        }
        ActionKind::MoveToTrash => {
            move_to_trash(target.path())?;
            Ok(ActionOutcome::MovedToTrash)
        }
    }
}

fn validate_target(target: &ActionTarget, action: ActionKind) -> Result<(), ActionError> {
    if target.path.as_os_str().is_empty() {
        return Err(ActionError::EmptyTarget);
    }
    // Copy actions are useful even for paths that have just disappeared, and
    // keeping them pure also makes it possible to copy a future destination.
    if matches!(action, ActionKind::CopyPath | ActionKind::CopyUri) {
        return Ok(());
    }
    fs::symlink_metadata(&target.path)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ActionError::TargetNotFound(target.path.clone())
            } else {
                ActionError::Io {
                    operation: "cannot inspect action target",
                    source: error,
                }
            }
        })
}

/// Return the exact text used by a clipboard-copy action without changing the
/// clipboard.  This is also useful to render an action preview.
pub fn copy_text_for(action: ActionKind, target: &ActionTarget) -> Result<String, ActionError> {
    if target.path.as_os_str().is_empty() {
        return Err(ActionError::EmptyTarget);
    }
    match action {
        ActionKind::CopyPath => Ok(target.path.to_string_lossy().into_owned()),
        ActionKind::CopyUri => path_to_file_uri(target.path()),
        _ => Err(ActionError::Unsupported {
            action,
            target: target.kind,
        }),
    }
}

/// Convert a path to a `file://` URI, escaping every byte that is not an RFC
/// 3986 unreserved character or a path separator.
///
/// Relative paths are first made absolute without canonicalising them.  This
/// keeps symlink spelling intact and allows copying a URI for a path that has
/// just disappeared.
#[cfg(not(windows))]
pub fn path_to_file_uri(path: &Path) -> Result<String, ActionError> {
    if path.as_os_str().is_empty() {
        return Err(ActionError::EmptyTarget);
    }
    let path = absolute_path(path)?;
    let mut output = String::from("file://");
    for byte in path_bytes(&path) {
        if byte == b'/' || is_unreserved(byte) {
            output.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    Ok(output)
}

#[cfg(windows)]
pub fn path_to_file_uri(path: &Path) -> Result<String, ActionError> {
    if path.as_os_str().is_empty() {
        return Err(ActionError::EmptyTarget);
    }
    let path = absolute_path(path)?;
    Ok(windows_file_uri(&path.to_string_lossy()))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_file_uri(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let (prefix, value) = if let Some(unc) = normalized.strip_prefix("//") {
        ("file://", unc)
    } else {
        ("file:///", normalized.as_str())
    };
    let mut output = String::from(prefix);
    for byte in value.bytes() {
        if byte == b'/' || byte == b':' || is_unreserved(byte) {
            output.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    output
}

fn absolute_path(path: &Path) -> Result<PathBuf, ActionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|source| ActionError::Io {
            operation: "cannot resolve the current directory",
            source,
        })
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(all(not(unix), not(windows)))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

#[cfg(not(windows))]
fn move_to_trash(path: &Path) -> Result<(), ActionError> {
    let path = absolute_path(path)?;
    let mut command = Command::new("gio");
    command.arg("trash").arg(path);
    run_command(&mut command, "gio")
}

#[cfg(windows)]
fn move_to_trash(path: &Path) -> Result<(), ActionError> {
    let path = absolute_path(path)?;
    trash::delete(&path).map_err(|error| ActionError::Io {
        operation: "cannot move target to the Recycle Bin",
        source: io::Error::other(error.to_string()),
    })
}

#[cfg(not(windows))]
fn run_command(command: &mut Command, program: &'static str) -> Result<(), ActionError> {
    let status = command.status().map_err(|source| ActionError::Io {
        operation: "cannot start external command",
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(ActionError::CommandFailed {
            program,
            status: status.code(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_actions_include_confirmed_trash_last() {
        let actions = actions_for_target(&ActionTarget::file("/tmp/report.txt"));
        assert_eq!(
            actions.iter().map(|action| action.kind).collect::<Vec<_>>(),
            vec![
                ActionKind::Open,
                ActionKind::Reveal,
                ActionKind::CopyPath,
                ActionKind::CopyUri,
                ActionKind::MoveToTrash,
            ]
        );
        let trash = actions.last().unwrap();
        assert!(trash.destructive);
        assert!(trash.requires_confirmation);
    }

    #[test]
    fn application_actions_never_offer_trash() {
        let actions = actions_for_target(&ActionTarget::application(
            "/usr/share/applications/example.desktop",
        ));
        assert_eq!(actions[0].title, "启动应用");
        assert!(
            actions
                .iter()
                .all(|action| action.kind != ActionKind::MoveToTrash)
        );
    }

    #[test]
    fn directory_actions_have_directory_specific_labels() {
        let actions = actions_for_target(&ActionTarget::directory("/tmp/project"));
        assert_eq!(actions[0].title, "打开目录");
        assert_eq!(actions[1].title, "打开上级目录");
    }

    #[test]
    #[cfg(not(windows))]
    fn file_uri_escapes_spaces_symbols_and_unicode() {
        let uri = path_to_file_uri(Path::new("/tmp/Alter 文档/a #1%.txt")).unwrap();
        assert_eq!(
            uri,
            "file:///tmp/Alter%20%E6%96%87%E6%A1%A3/a%20%231%25.txt"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn relative_file_uri_is_absolute_and_escaped() {
        let uri = path_to_file_uri(Path::new("folder/a b.txt")).unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("/folder/a%20b.txt"));
    }

    #[test]
    fn windows_file_uris_preserve_drive_and_unc_separators() {
        assert_eq!(
            windows_file_uri(r"C:\Users\Alter User\报告.txt"),
            "file:///C:/Users/Alter%20User/%E6%8A%A5%E5%91%8A.txt"
        );
        assert_eq!(
            windows_file_uri(r"\\server\share\a b.txt"),
            "file://server/share/a%20b.txt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_uri_preserves_non_utf8_paths_as_percent_encoded_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/tmp/a-\xff.bin"));
        assert_eq!(path_to_file_uri(path).unwrap(), "file:///tmp/a-%FF.bin");
    }

    #[test]
    fn unconfirmed_trash_is_rejected_without_touching_the_path() {
        let target = ActionTarget::file("/definitely/not/an/alter/test/file");
        let error = execute(ActionKind::MoveToTrash, &target).unwrap_err();
        assert!(matches!(error, ActionError::ConfirmationRequired));
    }

    #[test]
    fn application_desktop_entries_cannot_be_trashed() {
        let target = ActionTarget::application("/does/not/exist.desktop");
        let error = execute(ActionKind::MoveToTrash, &target).unwrap_err();
        assert!(matches!(
            error,
            ActionError::Unsupported {
                action: ActionKind::MoveToTrash,
                target: TargetKind::Application,
            }
        ));
    }

    #[test]
    fn copy_preview_rejects_non_copy_actions() {
        let target = ActionTarget::file("/tmp/example.txt");
        let error = copy_text_for(ActionKind::Open, &target).unwrap_err();
        assert!(matches!(
            error,
            ActionError::Unsupported {
                action: ActionKind::Open,
                target: TargetKind::File,
            }
        ));
    }
}
