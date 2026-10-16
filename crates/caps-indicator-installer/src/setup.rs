//! Per-user installer and uninstaller.
//!
//! This module is app-independent: the app-specific parts (payload,
//! registrations and texts) live in `main.rs` and are described by an
//! [`App`] implementation.
//!
//! ```text
//! <id>-setup.exe [--silent]                 install, update or repair
//! <id>-setup.exe --uninstall [--silent]     uninstall
//! ```
//!
//! Everything is installed for the current user, without administrator rights:
//!
//! | Item | Location |
//! | :--- | :--- |
//! | Program files | `%LOCALAPPDATA%\Programs\<id>` |
//! | Settings and data | `%LOCALAPPDATA%\<id>` |
//! | Start Menu folder | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\<display name>` |
//! | Settings > Apps entry | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<id>` |
//! | Uninstaller | `<program files>\<id>-uninstall.exe` (a copy of the installer) |

use crate::dialog::{task_dialog, Button, DialogIcon};
use crate::registry::{delete_reg_tree, get_reg_string, set_reg_dword, set_reg_string};
use crate::shortcut::create_shortcut;
use std::env;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

pub const PUBLISHER: &str = "stickylfs";

const BUTTON_INSTALL: i32 = 100;
const BUTTON_REPAIR: i32 = 101;
const BUTTON_UNINSTALL: i32 = 102;
const BUTTON_CANCEL: i32 = 103;
const BUTTON_PROCEED: i32 = 104;
const BUTTON_OK: i32 = 106;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Installer language, following the Windows display language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Es,
}

impl Language {
    pub fn detect() -> Self {
        let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        // Primary language identifier 0x0A is LANG_SPANISH (all regions).
        if lang_id & 0x3FF == 0x0A {
            Language::Es
        } else {
            Language::En
        }
    }

    /// Returns the text for this language.
    pub fn pick<'a>(self, en: &'a str, es: &'a str) -> &'a str {
        match self {
            Language::En => en,
            Language::Es => es,
        }
    }
}

/// A file copied to the program folder.
pub struct File {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

/// A shortcut created in the app's Start Menu folder.
pub struct Shortcut {
    pub name: String,
    pub target: PathBuf,
    pub description: String,
}

/// Locations used by the installation.
pub struct Paths {
    /// Program files: `%LOCALAPPDATA%\Programs\<id>`.
    pub install_dir: PathBuf,
    /// Settings and data: `%LOCALAPPDATA%\<id>`.
    pub data_dir: PathBuf,
    /// The application icon, installed as `app.ico`.
    pub icon: PathBuf,
}

/// Localized, app-specific texts.
pub struct Texts {
    /// Shown before installing: what the app does.
    pub description: String,
    /// Shown after installing: how to use the app.
    pub usage: String,
    /// Shown when confirming the uninstallation: what will be removed.
    pub uninstall_question: String,
    /// Shown after uninstalling: what has been removed.
    pub uninstall_done: String,
    /// Why a build for another CPU architecture does not work, for apps that
    /// load into other processes (`None`: it works, but slower).
    pub arch_reason: Option<String>,
}

pub trait App {
    /// Folder, registry and file name (e.g. `img-converter`).
    const ID: &'static str;
    /// Name shown to the user.
    const DISPLAY_NAME: &'static str;
    const VERSION: &'static str;
    /// Files copied to the program folder. One of them must be `app.ico`.
    const FILES: &'static [File];
    /// Executables stopped before their files are replaced or removed.
    const PROCESSES: &'static [&'static str] = &[];

    fn texts(lang: Language) -> Texts;

    /// App-specific setup after the files are copied (settings, shell
    /// integration, startup entry...).
    fn install(paths: &Paths, lang: Language) -> Result<(), String>;

    /// Undoes [`App::install`]. The program folder, the data folder, the Start
    /// Menu folder and the Settings > Apps entry are removed afterwards.
    fn uninstall(paths: &Paths);

    fn shortcuts(paths: &Paths, lang: Language) -> Vec<Shortcut>;

    /// Starts the app once an interactive installation has finished.
    fn launch(_paths: &Paths) {}

    /// Runs after the final dialog of an interactive installation (e.g. to
    /// suggest an optional Windows codec).
    fn after_install(_paths: &Paths, _lang: Language) {}
}

pub fn run<A: App>() {
    let args: Vec<String> = env::args().skip(1).map(|a| a.to_lowercase()).collect();
    let has_flag = |flags: &[&str]| args.iter().any(|a| flags.contains(&a.as_str()));
    let silent = has_flag(&["--silent", "-s", "/silent", "/s", "/verysilent"]);
    let running_as_uninstaller = env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n.contains("uninstall"));
    let lang = Language::detect();
    let paths = paths::<A>();

    if has_flag(&["--uninstall", "-u", "/uninstall"]) || running_as_uninstaller {
        if silent || confirm_uninstall::<A>(lang) {
            uninstall::<A>(&paths, silent, lang);
        }
        return;
    }

    if !check_architecture::<A>(silent, lang) {
        return;
    }

    let installed_version = installed_version::<A>(&paths);
    if silent {
        if let Err(e) = install::<A>(&paths, lang) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    let action = match &installed_version {
        Some(version) => prompt_installed::<A>(version, lang),
        None => prompt_install::<A>(lang),
    };
    match action {
        BUTTON_INSTALL | BUTTON_REPAIR => match install::<A>(&paths, lang) {
            Ok(()) => show_installed::<A>(&paths, installed_version.as_deref(), lang),
            Err(e) => {
                let title = setup_title::<A>(lang);
                let instruction = match lang {
                    Language::En => format!("{} could not be installed", A::DISPLAY_NAME),
                    Language::Es => format!("No se ha podido instalar {}", A::DISPLAY_NAME),
                };
                task_dialog(&title, &instruction, &e, &[], 0, DialogIcon::Error);
                std::process::exit(1);
            }
        },
        BUTTON_UNINSTALL => uninstall::<A>(&paths, false, lang),
        _ => {}
    }
}

fn local_app_data() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

fn paths<A: App>() -> Paths {
    let install_dir = local_app_data().join("Programs").join(A::ID);
    Paths {
        icon: install_dir.join("app.ico"),
        data_dir: local_app_data().join(A::ID),
        install_dir,
    }
}

fn start_menu_dir<A: App>() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|app_data| {
        PathBuf::from(app_data)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(A::DISPLAY_NAME)
    })
}

fn uninstall_key<A: App>() -> String {
    format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        A::ID
    )
}

fn uninstaller_name<A: App>() -> String {
    format!("{}-uninstall.exe", A::ID)
}

/// Version recorded in Settings > Apps, or `None` when the app is not installed.
fn installed_version<A: App>(paths: &Paths) -> Option<String> {
    let complete = A::FILES
        .iter()
        .all(|f| paths.install_dir.join(f.name).exists());
    complete.then(|| {
        get_reg_string(HKEY_CURRENT_USER, &uninstall_key::<A>(), "DisplayVersion")
            .unwrap_or_default()
    })
}

fn setup_title<A: App>(lang: Language) -> String {
    match lang {
        Language::En => format!("{} Setup", A::DISPLAY_NAME),
        Language::Es => format!("Instalación de {}", A::DISPLAY_NAME),
    }
}

fn confirm_uninstall<A: App>(lang: Language) -> bool {
    let name = A::DISPLAY_NAME;
    let (instruction, uninstall, cancel) = match lang {
        Language::En => (
            format!("Uninstall {name}"),
            format!(
                "Uninstall
Remove {name} from this PC."
            ),
            "Cancel
Exit without making any changes.",
        ),
        Language::Es => (
            format!("Desinstalar {name}"),
            format!(
                "Desinstalar
Quitar {name} de este equipo."
            ),
            "Cancelar
Salir sin realizar cambios.",
        ),
    };
    task_dialog(
        &setup_title::<A>(lang),
        &instruction,
        &A::texts(lang).uninstall_question,
        &[
            Button {
                id: BUTTON_UNINSTALL,
                text: &uninstall,
            },
            Button {
                id: BUTTON_CANCEL,
                text: cancel,
            },
        ],
        BUTTON_CANCEL,
        DialogIcon::App,
    ) == BUTTON_UNINSTALL
}

fn prompt_install<A: App>(lang: Language) -> i32 {
    let name = A::DISPLAY_NAME;
    let (instruction, install, cancel) = match lang {
        Language::En => (
            format!("Install {name} on your PC"),
            format!(
                "Install {name}\nInstalls into your user profile, no administrator rights required."
            ),
            "Cancel\nExit setup without making any changes.",
        ),
        Language::Es => (
            format!("Instalar {name} en tu equipo"),
            format!(
                "Instalar {name}\nSe instala en tu perfil de usuario, sin permisos de administrador."
            ),
            "Cancelar\nSalir del instalador sin realizar cambios.",
        ),
    };
    task_dialog(
        &setup_title::<A>(lang),
        &instruction,
        &A::texts(lang).description,
        &[
            Button {
                id: BUTTON_INSTALL,
                text: &install,
            },
            Button {
                id: BUTTON_CANCEL,
                text: cancel,
            },
        ],
        BUTTON_INSTALL,
        DialogIcon::App,
    )
}

fn prompt_installed<A: App>(installed_version: &str, lang: Language) -> i32 {
    let name = A::DISPLAY_NAME;
    let new_version = A::VERSION;
    let is_update = installed_version != new_version;
    let (instruction, content, repair, uninstall, cancel) = match lang {
        Language::En => (
            format!("{name} is already installed on your PC"),
            if is_update && !installed_version.is_empty() {
                format!("Installed version: {installed_version}. This setup installs version {new_version}.\n\nChoose what you would like to do:")
            } else {
                "Choose what you would like to do:".to_string()
            },
            if is_update {
                format!("Update to version {new_version}\nReplace the installed version and keep your settings.")
            } else {
                "Reinstall or repair\nCopy all components again and refresh the integration with Windows.".to_string()
            },
            format!("Uninstall\nRemove {name} from this PC."),
            "Cancel\nExit setup without making any changes.",
        ),
        Language::Es => (
            format!("{name} ya está instalado en tu equipo"),
            if is_update && !installed_version.is_empty() {
                format!("Versión instalada: {installed_version}. Este instalador contiene la versión {new_version}.\n\nElige qué quieres hacer:")
            } else {
                "Elige qué quieres hacer:".to_string()
            },
            if is_update {
                format!("Actualizar a la versión {new_version}\nSustituir la versión instalada y conservar la configuración.")
            } else {
                "Reinstalar o reparar\nVolver a copiar todos los componentes y actualizar la integración con Windows.".to_string()
            },
            format!("Desinstalar\nQuitar {name} de este equipo."),
            "Cancelar\nSalir del instalador sin realizar cambios.",
        ),
    };
    task_dialog(
        &setup_title::<A>(lang),
        &instruction,
        &content,
        &[
            Button {
                id: BUTTON_REPAIR,
                text: &repair,
            },
            Button {
                id: BUTTON_UNINSTALL,
                text: &uninstall,
            },
            Button {
                id: BUTTON_CANCEL,
                text: cancel,
            },
        ],
        BUTTON_REPAIR,
        DialogIcon::App,
    )
}

/// Processor architecture of Windows and of this build, as
/// `IMAGE_FILE_MACHINE_*` values.
fn machines() -> (u16, u16) {
    use windows::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
        IMAGE_FILE_MACHINE_I386,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

    let build = if cfg!(target_arch = "x86") {
        IMAGE_FILE_MACHINE_I386
    } else if cfg!(target_arch = "aarch64") {
        IMAGE_FILE_MACHINE_ARM64
    } else {
        IMAGE_FILE_MACHINE_AMD64
    };
    let mut process = IMAGE_FILE_MACHINE::default();
    let mut native = IMAGE_FILE_MACHINE::default();
    // An x64 process emulated on ARM64 is not WOW64, but the native machine is
    // still reported correctly.
    let native =
        match unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) } {
            Ok(()) if native.0 != 0 => native,
            _ => build,
        };
    (native.0, build.0)
}

/// Warns when this build does not match the Windows architecture.
fn check_architecture<A: App>(silent: bool, lang: Language) -> bool {
    use windows::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    };

    let (native, build) = machines();
    if native == build {
        return true;
    }
    let (situation_en, situation_es, recommended) = if native == IMAGE_FILE_MACHINE_ARM64.0 {
        (
            "Your computer has an ARM processor and you opened the version for Intel and AMD processors.",
            "Tu equipo tiene un procesador ARM y has abierto la versión para procesadores Intel y AMD.",
            format!("{}-setup-arm64.exe", A::ID),
        )
    } else if native == IMAGE_FILE_MACHINE_AMD64.0 && build != IMAGE_FILE_MACHINE_ARM64.0 {
        (
            "Your computer runs 64-bit Windows and you opened the 32-bit installer.",
            "Tu equipo funciona con Windows de 64 bits y has abierto el instalador de 32 bits.",
            format!("{}-setup.exe", A::ID),
        )
    } else {
        (
            "You opened the ARM version, but your computer uses an Intel or AMD processor.",
            "Has abierto la versión para procesadores ARM, pero tu equipo utiliza un procesador Intel o AMD.",
            format!("{}-setup.exe", A::ID),
        )
    };

    if silent {
        eprintln!(
            "Warning: this installer does not match the Windows architecture; {recommended} is recommended."
        );
        return true;
    }

    let texts = A::texts(lang);
    let critical = texts.arch_reason.is_some();
    let reason = texts.arch_reason.unwrap_or_else(|| {
        lang.pick(
            "It works, but the version that matches your system is faster",
            "Funciona, pero la versión que coincide con tu sistema es más rápida",
        )
        .to_string()
    });
    let content = match lang {
        Language::En => format!(
            "{situation_en}\n\n{reason} ({recommended}).\n\nDo you want to continue anyway?"
        ),
        Language::Es => format!(
            "{situation_es}\n\n{reason} ({recommended}).\n\n¿Quieres continuar de todos modos?"
        ),
    };
    let (instruction, proceed, cancel) = match (lang, critical) {
        (Language::En, true) => (
            "This version does not match your PC",
            "Continue anyway\nInstall this version on my computer.",
            "Cancel (recommended)\nClose setup and download the matching version.",
        ),
        (Language::En, false) => (
            "There is a better version for your PC",
            "Continue\nInstall this version on my computer.",
            "Cancel\nClose setup and download the matching version.",
        ),
        (Language::Es, true) => (
            "Esta versión no es la adecuada para tu equipo",
            "Continuar de todos modos\nInstalar esta versión en mi equipo.",
            "Cancelar (recomendado)\nCerrar el instalador y descargar la versión adecuada.",
        ),
        (Language::Es, false) => (
            "Hay una versión más adecuada para tu equipo",
            "Continuar\nInstalar esta versión en mi equipo.",
            "Cancelar\nCerrar el instalador y descargar la versión adecuada.",
        ),
    };
    task_dialog(
        &setup_title::<A>(lang),
        instruction,
        &content,
        &[
            Button {
                id: BUTTON_CANCEL,
                text: cancel,
            },
            Button {
                id: BUTTON_PROCEED,
                text: proceed,
            },
        ],
        if critical {
            BUTTON_CANCEL
        } else {
            BUTTON_PROCEED
        },
        DialogIcon::Warning,
    ) == BUTTON_PROCEED
}

fn install<A: App>(paths: &Paths, lang: Language) -> Result<(), String> {
    if let Some(file) = A::FILES.iter().find(|f| f.bytes.is_empty()) {
        return Err(format!(
            "This installer was built without its payload ({} is missing). Build the \
             application before the installer.",
            file.name
        ));
    }

    stop_processes(A::PROCESSES);

    let install_dir = &paths.install_dir;
    fs::create_dir_all(install_dir)
        .map_err(|e| format!("Cannot create {}: {}", install_dir.display(), e))?;

    // Remove files left over by earlier upgrades of locked files.
    if let Ok(entries) = fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains(".old.") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    // 1. Program files, and a copy of this installer as the uninstaller
    for file in A::FILES {
        safe_write_file(&install_dir.join(file.name), file.bytes)
            .map_err(|e| format!("Cannot write {}: {}", file.name, e))?;
    }
    let uninstaller_path = install_dir.join(uninstaller_name::<A>());
    if let Ok(current_exe) = env::current_exe() {
        if current_exe != uninstaller_path {
            let _ = fs::copy(&current_exe, &uninstaller_path);
        }
    }

    // 2. App-specific setup
    A::install(paths, lang)?;

    // 3. Start Menu folder (recreated so renamed shortcuts do not linger)
    if let Some(start_menu_dir) = start_menu_dir::<A>() {
        let _ = fs::remove_dir_all(&start_menu_dir);
        let _ = fs::create_dir_all(&start_menu_dir);
        for shortcut in A::shortcuts(paths, lang) {
            let _ = create_shortcut(
                &shortcut.target,
                &start_menu_dir.join(format!("{}.lnk", shortcut.name)),
                &shortcut.description,
                Some(&paths.icon),
            );
        }
    }

    // 4. Settings > Apps entry
    let key = uninstall_key::<A>();
    let uninstaller = format!("\"{}\"", uninstaller_path.display());
    let estimated_kb = (A::FILES.iter().map(|f| f.bytes.len()).sum::<usize>() / 1024) as u32;
    let values = [
        ("DisplayName", A::DISPLAY_NAME.to_string()),
        ("DisplayIcon", paths.icon.display().to_string()),
        ("DisplayVersion", A::VERSION.to_string()),
        ("Publisher", PUBLISHER.to_string()),
        ("InstallLocation", install_dir.display().to_string()),
        ("UninstallString", uninstaller.clone()),
        ("QuietUninstallString", format!("{uninstaller} --silent")),
    ];
    for (name, value) in values {
        let _ = set_reg_string(HKEY_CURRENT_USER, &key, Some(name), &value);
    }
    let _ = set_reg_dword(HKEY_CURRENT_USER, &key, "NoModify", 1);
    let _ = set_reg_dword(HKEY_CURRENT_USER, &key, "NoRepair", 1);
    let _ = set_reg_dword(HKEY_CURRENT_USER, &key, "EstimatedSize", estimated_kb);

    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    Ok(())
}

fn show_installed<A: App>(paths: &Paths, previous_version: Option<&str>, lang: Language) {
    let name = A::DISPLAY_NAME;
    let instruction = match (lang, previous_version) {
        (Language::En, None) => format!("{name} has been installed successfully"),
        (Language::En, Some(v)) if v == A::VERSION => {
            format!("{name} has been repaired successfully")
        }
        (Language::En, Some(_)) => format!("{name} has been updated successfully"),
        (Language::Es, None) => format!("{name} se ha instalado correctamente"),
        (Language::Es, Some(v)) if v == A::VERSION => {
            format!("{name} se ha reparado correctamente")
        }
        (Language::Es, Some(_)) => format!("{name} se ha actualizado correctamente"),
    };

    let texts = A::texts(lang);
    let buttons = [Button {
        id: BUTTON_OK,
        text: lang.pick("OK", "Aceptar"),
    }];

    A::launch(paths);
    task_dialog(
        &setup_title::<A>(lang),
        &instruction,
        &texts.usage,
        &buttons,
        BUTTON_OK,
        DialogIcon::App,
    );
    A::after_install(paths, lang);
}

fn uninstall<A: App>(paths: &Paths, silent: bool, lang: Language) {
    stop_processes(A::PROCESSES);

    // 1. App-specific registrations, then the Start Menu folder and Settings > Apps entry
    A::uninstall(paths);
    if let Some(start_menu_dir) = start_menu_dir::<A>() {
        let _ = fs::remove_dir_all(start_menu_dir);
    }
    let _ = delete_reg_tree(HKEY_CURRENT_USER, &uninstall_key::<A>());
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }

    // 2. Program files (a shell extension may still be loaded) and user data
    for file in A::FILES {
        remove_or_move_away(&paths.install_dir.join(file.name));
    }
    let _ = fs::remove_dir_all(&paths.data_dir);

    if !silent {
        let instruction = match lang {
            Language::En => format!("{} has been uninstalled successfully", A::DISPLAY_NAME),
            Language::Es => format!("{} se ha desinstalado correctamente", A::DISPLAY_NAME),
        };
        task_dialog(
            &setup_title::<A>(lang),
            &instruction,
            &A::texts(lang).uninstall_done,
            &[],
            0,
            DialogIcon::App,
        );
    }

    // 3. The running uninstaller cannot delete itself: let a short-lived hidden
    //    shell remove it and the program folder after we exit.
    schedule_install_dir_removal::<A>(&paths.install_dir);
}

/// Writes a file that may be locked (e.g. a shell extension DLL loaded by
/// Explorer): a loaded file cannot be overwritten but can be renamed away.
fn safe_write_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::write(path, content).is_ok() {
        return Ok(());
    }
    let old_path = path.with_extension(format!("old.{}", std::process::id()));
    fs::rename(path, &old_path)?;
    fs::write(path, content)
}

/// Deletes a file, moving it to %TEMP% when it is locked.
fn remove_or_move_away(path: &Path) {
    if !path.exists() || fs::remove_file(path).is_ok() {
        return;
    }
    if let Some(name) = path.file_name() {
        let dest = env::temp_dir().join(format!(
            "{}-{}.old",
            name.to_string_lossy(),
            std::process::id()
        ));
        let _ = fs::rename(path, dest);
    }
}

/// Terminates running instances of `exe_names` and waits for them to exit.
fn stop_processes(exe_names: &[&str]) {
    if exe_names.is_empty() {
        return;
    }
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return;
        };
        let current_pid = GetCurrentProcessId();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if entry.th32ProcessID != current_pid
                && exe_names.iter().any(|n| name.eq_ignore_ascii_case(n))
            {
                if let Ok(process) = OpenProcess(
                    PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                    false,
                    entry.th32ProcessID,
                ) {
                    if TerminateProcess(process, 1).is_ok() {
                        WaitForSingleObject(process, 3000);
                    }
                    let _ = CloseHandle(process);
                }
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
}

fn schedule_install_dir_removal<A: App>(install_dir: &Path) {
    // Safety net: never schedule a recursive delete outside our own folder.
    if install_dir.file_name().map(|n| n != A::ID).unwrap_or(true) || !install_dir.exists() {
        return;
    }
    let script = format!(
        "ping 127.0.0.1 -n 3 > nul & rmdir /s /q \"{}\"",
        install_dir.display()
    );
    let _ = Command::new("cmd.exe")
        .arg("/c")
        .raw_arg(&script)
        .current_dir(env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_matches_native_machine_or_is_emulated() {
        let (native, build) = machines();
        assert_ne!(native, 0);
        assert_ne!(build, 0);
    }
}
