//! # caps-indicator
//!
//! Lightweight, offline Caps Lock indicator for the Windows notification area.
//!
//! Monitors the system toggle state of `VK_CAPITAL` via direct Win32 queries and
//! reflects it with an icon and tooltip in the Windows notification area.

#![windows_subsystem = "windows"]

use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS,
    HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetSystemMetricsForDpi, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, KillTimer, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, TrackPopupMenuEx,
    TranslateMessage, CW_USEDEFAULT, GWLP_USERDATA, HICON, LR_DEFAULTCOLOR, MB_ICONERROR,
    MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MF_CHECKED, MF_SEPARATOR, MF_STRING, SM_CXSMICON,
    SM_CYSMICON, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_COMMAND, WM_DESTROY,
    WM_ENDSESSION, WM_NCDESTROY, WM_NULL, WM_QUERYENDSESSION, WM_RBUTTONUP, WM_TIMER, WM_USER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

const WM_TRAYICON: u32 = WM_USER + 1;
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 200;
const IDM_EXIT: usize = 1001;
const IDM_STARTUP: usize = 1002;
const TRAY_ICON_ID: u32 = 1;

/// `HKCU` subkey and value name of the "start with Windows" entry. Its presence
/// is the whole setting, so the app needs no config file.
const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE_NAME: &str = "caps-indicator";

/// Primary language identifier for Spanish (`LANG_SPANISH`).
const LANG_SPANISH: u16 = 0x0A;

static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

const ICON_OFF_BYTES: &[u8] = include_bytes!("../../../assets/caps-off.ico");
const ICON_ON_BYTES: &[u8] = include_bytes!("../../../assets/caps-on.ico");

/// Encodes a Rust string as a NUL-terminated UTF-16 buffer for Win32 APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// User-facing strings, resolved once from the Windows UI language.
struct Strings {
    exit: &'static str,
    start_with_windows: &'static str,
    startup_entry_failed: &'static str,
    tooltip_on: &'static str,
    tooltip_off: &'static str,
    already_running: &'static str,
    startup_failed: &'static str,
}

impl Strings {
    fn detect() -> Self {
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        if lang_id & 0x3FF == LANG_SPANISH {
            Strings {
                exit: "Salir",
                start_with_windows: "Iniciar con Windows",
                startup_entry_failed: "No se ha podido cambiar el inicio con Windows",
                tooltip_on: "Bloq Mayús: activado",
                tooltip_off: "Bloq Mayús: desactivado",
                already_running: "caps-indicator ya se está ejecutando en el área de notificación.",
                startup_failed: "caps-indicator no se ha podido iniciar",
            }
        } else {
            Strings {
                exit: "Exit",
                start_with_windows: "Start with Windows",
                startup_entry_failed: "Could not change Start with Windows",
                tooltip_on: "Caps Lock: ON",
                tooltip_off: "Caps Lock: OFF",
                already_running: "caps-indicator is already running in the notification area.",
                startup_failed: "caps-indicator could not start",
            }
        }
    }
}

/// Reads a little-endian `u32` at `offset`, returning `None` when out of bounds.
fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Parses an `.ico` container and returns the image payload whose dimensions are
/// closest to `desired_cx` x `desired_cy` (ties keep the first, i.e. smallest, entry).
fn select_icon_image(bytes: &[u8], desired_cx: i32, desired_cy: i32) -> Option<&[u8]> {
    if bytes.len() < 6 {
        return None;
    }

    let reserved = u16::from_le_bytes([bytes[0], bytes[1]]);
    let icon_type = u16::from_le_bytes([bytes[2], bytes[3]]);
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if reserved != 0 || icon_type != 1 || count == 0 {
        return None;
    }

    let mut best: Option<(i32, &[u8])> = None;
    for i in 0..count {
        let entry = 6 + i * 16;
        if entry + 16 > bytes.len() {
            break;
        }

        // A stored dimension of 0 means 256 px.
        let w = match bytes[entry] {
            0 => 256,
            v => v as i32,
        };
        let h = match bytes[entry + 1] {
            0 => 256,
            v => v as i32,
        };

        let size = read_u32_le(bytes, entry + 8)? as usize;
        let offset = read_u32_le(bytes, entry + 12)? as usize;
        let Some(image) = offset
            .checked_add(size)
            .and_then(|end| bytes.get(offset..end))
        else {
            continue;
        };

        let diff = (w - desired_cx).abs() + (h - desired_cy).abs();
        if best.is_none_or(|(best_diff, _)| diff < best_diff) {
            best = Some((diff, image));
        }
    }

    best.map(|(_, image)| image)
}

/// Creates an `HICON` from an embedded `.ico` buffer, sized for the system small
/// icon metric (`SM_CXSMICON`) at the current system DPI.
fn load_icon_from_ico_bytes(bytes: &[u8]) -> Option<HICON> {
    let (cx, cy) = unsafe {
        let dpi = GetDpiForSystem();
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            GetSystemMetricsForDpi(SM_CYSMICON, dpi),
        )
    };

    let image = select_icon_image(bytes, cx, cy)?;
    let hicon = unsafe {
        CreateIconFromResourceEx(
            image.as_ptr(),
            image.len() as u32,
            1, // TRUE = icon (not cursor)
            0x0003_0000,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        )
    };

    if hicon.is_null() {
        None
    } else {
        Some(hicon)
    }
}

/// Queries the real-time global toggle state of `VK_CAPITAL` (Caps Lock).
///
/// Returns `true` if Caps Lock is active (toggled on), `false` otherwise.
#[inline]
fn caps_lock_enabled() -> bool {
    let state = unsafe { GetKeyState(VK_CAPITAL as i32) };
    (state & 1) != 0
}

/// Returns `true` when the `HKCU\...\Run` entry for caps-indicator exists.
fn startup_enabled() -> bool {
    let subkey = wide(RUN_SUBKEY);
    let name = wide(RUN_VALUE_NAME);
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ) == ERROR_SUCCESS
    }
}

/// Adds or removes the `HKCU\...\Run` entry that starts this exe at sign-in.
fn set_startup(enable: bool) -> Result<(), u32> {
    let subkey = wide(RUN_SUBKEY);
    let name = wide(RUN_VALUE_NAME);
    let code = unsafe {
        if enable {
            let exe = std::env::current_exe().map_err(|e| e.raw_os_error().unwrap_or(1) as u32)?;
            let command = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * mem::size_of::<u16>()) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr())
        }
    };
    // Disabling an entry that is already gone is not an error.
    if code == ERROR_SUCCESS || (!enable && code == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(code)
    }
}

/// Copies `text` into the fixed-size `szTip` buffer, truncating if needed.
fn set_tooltip(nid: &mut NOTIFYICONDATAW, text: &str) {
    // Build the buffer locally: `NOTIFYICONDATAW` is packed on x86, so its
    // fields cannot be borrowed in place.
    let mut tip = [0u16; 128];
    for (dst, src) in tip.iter_mut().zip(text.encode_utf16().take(127)) {
        *dst = src;
    }
    nid.szTip = tip;
}

struct AppState {
    icon_off: HICON,
    icon_on: HICON,
    current_state: bool,
    tray_added: bool,
    nid: NOTIFYICONDATAW,
    strings: Strings,
}

impl AppState {
    /// Fills the notification data for the given Caps Lock state.
    fn apply_state(&mut self, is_on: bool) {
        self.current_state = is_on;
        self.nid.hIcon = if is_on { self.icon_on } else { self.icon_off };
        let tip = if is_on {
            self.strings.tooltip_on
        } else {
            self.strings.tooltip_off
        };
        set_tooltip(&mut self.nid, tip);
        // Always resend every flag: a NIM_ADD without NIF_MESSAGE would silently
        // disable the context menu after an Explorer restart.
        self.nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    }

    /// (Re-)registers the tray icon, e.g. at startup or after Explorer restarts.
    fn add_tray_icon(&mut self) {
        self.apply_state(caps_lock_enabled());
        self.tray_added = unsafe { Shell_NotifyIconW(NIM_ADD, &self.nid) } != 0;
    }

    fn remove_tray_icon(&mut self) {
        if self.tray_added {
            unsafe { Shell_NotifyIconW(NIM_DELETE, &self.nid) };
            self.tray_added = false;
        }
    }

    fn update_status(&mut self) {
        let is_on = caps_lock_enabled();
        if !self.tray_added {
            self.add_tray_icon();
        } else if is_on != self.current_state {
            self.apply_state(is_on);
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.nid) };
        }
    }
}

unsafe fn show_context_menu(hwnd: HWND, state: &AppState) {
    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    let text_startup = wide(state.strings.start_with_windows);
    let startup_flags = if startup_enabled() {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    };
    AppendMenuW(hmenu, startup_flags, IDM_STARTUP, text_startup.as_ptr());
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    let text_exit = wide(state.strings.exit);
    AppendMenuW(hmenu, MF_STRING, IDM_EXIT, text_exit.as_ptr());

    let mut cursor_pos = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pos);

    // Recommended Win32 focus handling sequence for tray popup menus (KB135788)
    SetForegroundWindow(hwnd);
    TrackPopupMenuEx(
        hmenu,
        TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        cursor_pos.x,
        cursor_pos.y,
        hwnd,
        ptr::null(),
    );
    DestroyMenu(hmenu);
    PostMessageW(hwnd, WM_NULL, 0, 0);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;

    let taskbar_created = WM_TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        if let Some(state) = state_ptr.as_mut() {
            state.add_tray_icon();
        }
        return 0;
    }

    match msg {
        WM_TIMER => {
            if wparam == TIMER_ID {
                if let Some(state) = state_ptr.as_mut() {
                    state.update_status();
                }
            }
            0
        }
        WM_TRAYICON => {
            let event = (lparam & 0xFFFF) as u32;
            if event == WM_RBUTTONUP {
                if let Some(state) = state_ptr.as_ref() {
                    show_context_menu(hwnd, state);
                }
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xFFFF {
                IDM_EXIT => {
                    DestroyWindow(hwnd);
                }
                IDM_STARTUP if set_startup(!startup_enabled()).is_err() => {
                    if let Some(state) = state_ptr.as_ref() {
                        show_message(state.strings.startup_entry_failed, MB_ICONERROR);
                    }
                }
                _ => {}
            }
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wparam != 0 {
                KillTimer(hwnd, TIMER_ID);
                if let Some(state) = state_ptr.as_mut() {
                    state.remove_tray_icon();
                }
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            if let Some(state) = state_ptr.as_mut() {
                state.remove_tray_icon();
            }
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut AppState;
            if !ptr.is_null() {
                let state = Box::from_raw(ptr);
                DestroyIcon(state.icon_off);
                DestroyIcon(state.icon_on);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        // Must run before any window or icon is created so metrics match the real DPI.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Single instance per user session: a second copy would add a duplicate tray icon.
    let mutex_name = wide("Local\\caps-indicator-single-instance");
    let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
    let strings = Strings::detect();
    if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if !mutex.is_null() {
            unsafe { CloseHandle(mutex) };
            show_message(strings.already_running, MB_ICONINFORMATION);
        }
        return;
    }

    let startup_failed = strings.startup_failed;
    if let Err(code) = run(strings) {
        let text = format!("{startup_failed} (error {code}).");
        show_message(&text, MB_ICONERROR);
    }

    unsafe { CloseHandle(mutex) };
}

/// Shows a message box titled "caps-indicator".
fn show_message(text: &str, icon: MESSAGEBOX_STYLE) {
    let text = wide(text);
    let title = wide("caps-indicator");
    unsafe { MessageBoxW(ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

/// Creates the hidden window and the tray icon, then runs the message loop.
/// Returns the Win32 error code when the app cannot start.
fn run(strings: Strings) -> Result<(), u32> {
    let last_error = || unsafe { GetLastError() };
    let Some(icon_off) = load_icon_from_ico_bytes(ICON_OFF_BYTES) else {
        return Err(last_error());
    };
    let Some(icon_on) = load_icon_from_ico_bytes(ICON_ON_BYTES) else {
        let code = last_error();
        unsafe { DestroyIcon(icon_off) };
        return Err(code);
    };

    let destroy_icons = || unsafe {
        DestroyIcon(icon_off);
        DestroyIcon(icon_on);
    };

    let taskbar_msg_name = wide("TaskbarCreated");
    let msg_id = unsafe { RegisterWindowMessageW(taskbar_msg_name.as_ptr()) };
    WM_TASKBAR_CREATED.store(msg_id, Ordering::Relaxed);

    let class_name = wide("CapsLockIndicatorClass");
    let h_instance = unsafe { GetModuleHandleW(ptr::null()) };

    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: h_instance as HINSTANCE,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    if unsafe { RegisterClassW(&wc) } == 0 {
        let code = last_error();
        destroy_icons();
        return Err(code);
    }

    // A hidden top-level window (not HWND_MESSAGE): message-only windows never
    // receive the broadcast `TaskbarCreated` message.
    let window_title = wide("CapsLockIndicator");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ptr::null_mut(),
            ptr::null_mut(),
            h_instance as HINSTANCE,
            ptr::null(),
        )
    };

    if hwnd.is_null() {
        let code = last_error();
        destroy_icons();
        return Err(code);
    }

    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    nid.uCallbackMessage = WM_TRAYICON;

    let mut state = Box::new(AppState {
        icon_off,
        icon_on,
        current_state: false,
        tray_added: false,
        nid,
        strings,
    });
    state.add_tray_icon();

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as _);
        SetTimer(hwnd, TIMER_ID, TIMER_INTERVAL_MS, None);
    }

    let mut msg = unsafe { mem::zeroed() };
    unsafe {
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal `.ico` container holding one dummy payload per size.
    fn build_ico(sizes: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0];
        out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + sizes.len() * 16;
        for &size in sizes {
            out.extend_from_slice(&[size, size, 0, 0, 1, 0, 32, 0]);
            out.extend_from_slice(&4u32.to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += 4;
        }
        for &size in sizes {
            out.extend_from_slice(&[size; 4]);
        }
        out
    }

    #[test]
    fn test_load_embedded_icons() {
        for bytes in [ICON_OFF_BYTES, ICON_ON_BYTES] {
            let icon = load_icon_from_ico_bytes(bytes).expect("embedded icon must load");
            unsafe { DestroyIcon(icon) };
        }
    }

    #[test]
    fn test_selects_closest_size() {
        let ico = build_ico(&[16, 32, 48, 0]);
        assert_eq!(select_icon_image(&ico, 16, 16), Some(&[16u8; 4][..]));
        assert_eq!(select_icon_image(&ico, 24, 24), Some(&[16u8; 4][..]));
        assert_eq!(select_icon_image(&ico, 40, 40), Some(&[32u8; 4][..]));
        assert_eq!(select_icon_image(&ico, 300, 300), Some(&[0u8; 4][..]));
    }

    #[test]
    fn test_rejects_invalid_ico_bytes() {
        assert!(select_icon_image(&[], 16, 16).is_none());
        assert!(select_icon_image(&[0, 0, 0, 0], 16, 16).is_none());
        // Cursor type (2) instead of icon (1).
        assert!(select_icon_image(&[0, 0, 2, 0, 1, 0], 16, 16).is_none());
        // Truncated directory entry.
        assert!(select_icon_image(&[0, 0, 1, 0, 1, 0, 16, 16], 16, 16).is_none());
    }

    #[test]
    fn test_skips_out_of_bounds_payload() {
        let mut ico = build_ico(&[16, 32]);
        // Point the 16 px entry past the end of the buffer.
        ico[6 + 12..6 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(select_icon_image(&ico, 16, 16), Some(&[32u8; 4][..]));
    }

    #[test]
    fn test_tooltip_truncates() {
        let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        set_tooltip(&mut nid, &"x".repeat(500));
        let tip = nid.szTip;
        assert_eq!(tip[126], 'x' as u16);
        assert_eq!(tip[127], 0);
    }
}
