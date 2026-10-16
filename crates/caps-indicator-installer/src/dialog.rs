//! Native TaskDialog prompts with a MessageBox fallback.

use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON, TASKDIALOG_FLAGS,
    TASKDIALOG_NOTIFICATIONS, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDF_USE_COMMAND_LINKS, TDF_USE_HICON_MAIN, TDN_CREATED, TD_ERROR_ICON, TD_INFORMATION_ICON,
    TD_WARNING_ICON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    LoadImageW, MessageBoxW, SendMessageW, HICON, ICON_BIG, ICON_SMALL, IDCANCEL, IDNO, IDOK,
    IDYES, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL, MESSAGEBOX_STYLE, WM_SETICON,
};

type TaskDialogIndirectFn = unsafe extern "system" fn(
    p_task_config: *const TASKDIALOGCONFIG,
    pn_button: *mut i32,
    pn_radio_button: *mut i32,
    pf_verification_flag_checked: *mut BOOL,
) -> windows_core::HRESULT;

/// Button ID returned when the dialog is closed with Esc or the close button.
pub const CANCELLED: i32 = IDCANCEL.0;

// This module is app-independent, so not every variant is used by every
// installer.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogIcon {
    /// The application icon embedded in the executable.
    App,
    Information,
    Warning,
    Error,
}

/// A dialog button. With command links, text after the first `\n` is shown as
/// a smaller description line.
pub struct Button<'a> {
    pub id: i32,
    pub text: &'a str,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `TaskDialogIndirect` lives in Common Controls v6, which the executable
/// manifest requests (see `build.rs`).
fn task_dialog_fn() -> Option<TaskDialogIndirectFn> {
    unsafe {
        let module = LoadLibraryW(w!("comctl32.dll")).ok()?;
        let proc = GetProcAddress(module, s!("TaskDialogIndirect"))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            TaskDialogIndirectFn,
        >(proc))
    }
}

#[allow(clippy::manual_dangling_ptr)]
fn app_icon() -> Option<HICON> {
    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let handle = LoadImageW(
            hinstance,
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        )
        .ok()?;
        Some(HICON(handle.0))
    }
}

/// Sets the application icon on the dialog's title bar and taskbar button.
unsafe extern "system" fn set_window_icon(
    hwnd: HWND,
    notification: TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    icon: isize,
) -> windows_core::HRESULT {
    if notification == TDN_CREATED && icon != 0 {
        for size in [ICON_SMALL, ICON_BIG] {
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(size as usize), LPARAM(icon));
        }
    }
    windows_core::HRESULT(0)
}

/// Shows a TaskDialog and returns the ID of the chosen button ([`CANCELLED`] when
/// dismissed). With no buttons, a single OK button is shown.
pub fn task_dialog(
    title: &str,
    instruction: &str,
    content: &str,
    buttons: &[Button],
    default_button: i32,
    icon: DialogIcon,
) -> i32 {
    task_dialog_with_details(
        title,
        instruction,
        content,
        "",
        buttons,
        default_button,
        icon,
    )
}

/// Like [`task_dialog`], with `details` in a collapsible "See details" section
/// (hidden when empty).
pub fn task_dialog_with_details(
    title: &str,
    instruction: &str,
    content: &str,
    details: &str,
    buttons: &[Button],
    default_button: i32,
    icon: DialogIcon,
) -> i32 {
    if let Some(func) = task_dialog_fn() {
        let title_w = wide(title);
        let instruction_w = wide(instruction);
        let content_w = wide(content);
        let details_w = wide(details);
        let labels: Vec<Vec<u16>> = buttons.iter().map(|b| wide(b.text)).collect();
        let td_buttons: Vec<TASKDIALOG_BUTTON> = buttons
            .iter()
            .zip(&labels)
            .map(|(b, label)| TASKDIALOG_BUTTON {
                nButtonID: b.id,
                pszButtonText: PCWSTR(label.as_ptr()),
            })
            .collect();

        let mut flags: TASKDIALOG_FLAGS =
            TDF_POSITION_RELATIVE_TO_WINDOW | TDF_ALLOW_DIALOG_CANCELLATION;
        if buttons.iter().any(|b| b.text.contains('\n')) {
            flags |= TDF_USE_COMMAND_LINKS;
        }

        let hicon = app_icon();
        let main_icon = match (icon, hicon) {
            (DialogIcon::App, Some(h)) => {
                flags |= TDF_USE_HICON_MAIN;
                TASKDIALOGCONFIG_0 { hMainIcon: h }
            }
            (DialogIcon::Warning, _) => TASKDIALOGCONFIG_0 {
                pszMainIcon: TD_WARNING_ICON,
            },
            (DialogIcon::Error, _) => TASKDIALOGCONFIG_0 {
                pszMainIcon: TD_ERROR_ICON,
            },
            _ => TASKDIALOGCONFIG_0 {
                pszMainIcon: TD_INFORMATION_ICON,
            },
        };

        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            dwFlags: flags,
            pszWindowTitle: PCWSTR(title_w.as_ptr()),
            pszMainInstruction: PCWSTR(instruction_w.as_ptr()),
            pszContent: PCWSTR(content_w.as_ptr()),
            pszExpandedInformation: if details.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(details_w.as_ptr())
            },
            cButtons: td_buttons.len() as u32,
            pButtons: if td_buttons.is_empty() {
                std::ptr::null()
            } else {
                td_buttons.as_ptr()
            },
            nDefaultButton: default_button,
            Anonymous1: main_icon,
            pfCallback: hicon.map(|_| set_window_icon as _),
            lpCallbackData: hicon.map_or(0, |h| h.0 as isize),
            ..Default::default()
        };

        let mut selected = CANCELLED;
        let hr = unsafe {
            func(
                &config,
                &mut selected,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if hr.is_ok() {
            return if buttons.is_empty() { IDOK.0 } else { selected };
        }
    }

    let content = if details.is_empty() {
        content.to_string()
    } else {
        format!(
            "{content}

{details}"
        )
    };
    message_box_fallback(title, instruction, &content, buttons, icon)
}

/// Maps up to three buttons onto Yes / No / Cancel when TaskDialog is unavailable.
fn message_box_fallback(
    title: &str,
    instruction: &str,
    content: &str,
    buttons: &[Button],
    icon: DialogIcon,
) -> i32 {
    let icon_style = match icon {
        DialogIcon::Warning => MB_ICONWARNING,
        DialogIcon::Error => MB_ICONERROR,
        _ if buttons.len() > 1 => MB_ICONQUESTION,
        _ => MB_ICONINFORMATION,
    };
    let style = match buttons.len() {
        0 | 1 => MB_OK,
        2 => MB_YESNO,
        _ => MB_YESNOCANCEL,
    };

    let mut text = format!("{}\n\n{}", instruction, content);
    for (key, button) in ["Yes", "No", "Cancel"]
        .iter()
        .zip(buttons)
        .filter(|_| buttons.len() > 1)
    {
        let label = button.text.lines().next().unwrap_or_default();
        text.push_str(&format!("\n{}: {}", key, label));
    }

    let choice = message_box(title, &text, style | icon_style);
    let index = match choice {
        IDYES | IDOK => 0,
        IDNO => 1,
        _ => return CANCELLED,
    };
    buttons.get(index).map_or(IDOK.0, |b| b.id)
}

/// Shows a plain MessageBox and returns the raw result.
pub fn message_box(
    title: &str,
    text: &str,
    style: MESSAGEBOX_STYLE,
) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    let title_w = wide(title);
    let text_w = wide(text);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text_w.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            style,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_dialog_is_available() {
        assert!(
            task_dialog_fn().is_some(),
            "TaskDialogIndirect must resolve through Common Controls v6"
        );
    }
}
