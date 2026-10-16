use std::path::Path;
use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Balances a successful `CoInitializeEx` call, including on early returns.
struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Creates a `.lnk` shortcut at `shortcut_path` pointing to `target`.
pub fn create_shortcut(
    target: &Path,
    shortcut_path: &Path,
    description: &str,
    icon_path: Option<&Path>,
) -> windows::core::Result<()> {
    unsafe {
        let _com = ComGuard(CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok());

        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;

        let target_w = wide(&target.to_string_lossy());
        shell_link.SetPath(PCWSTR(target_w.as_ptr()))?;

        if let Some(parent) = target.parent() {
            let dir_w = wide(&parent.to_string_lossy());
            shell_link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))?;
        }

        if let Some(icon) = icon_path {
            let icon_w = wide(&icon.to_string_lossy());
            let _ = shell_link.SetIconLocation(PCWSTR(icon_w.as_ptr()), 0);
        }

        let desc_w = wide(description);
        shell_link.SetDescription(PCWSTR(desc_w.as_ptr()))?;

        let persist_file: IPersistFile = shell_link.cast()?;
        let shortcut_w = wide(&shortcut_path.to_string_lossy());
        persist_file.Save(PCWSTR(shortcut_w.as_ptr()), true)
    }
}
