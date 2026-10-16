#![windows_subsystem = "windows"]

//! `caps-indicator-setup.exe`: self-contained per-user installer and uninstaller.
//!
//! The generic install flow lives in `setup.rs`; this file adds the startup
//! entry and the caps-indicator texts.

mod dialog;
mod registry;
mod setup;
mod shortcut;

use registry::{delete_reg_value, get_reg_string, set_reg_string};
use setup::{App, File, Language, Paths, Shortcut, Texts};
use std::process::Command;
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

const APP_EXE_NAME: &str = "caps-indicator.exe";
const RUN_REG_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

struct CapsIndicator;

impl App for CapsIndicator {
    const ID: &'static str = "caps-indicator";
    const DISPLAY_NAME: &'static str = "caps-indicator";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const FILES: &'static [File] = &[
        File {
            name: APP_EXE_NAME,
            bytes: include_bytes!(concat!(env!("OUT_DIR"), "/caps-indicator.exe")),
        },
        File {
            name: "app.ico",
            bytes: include_bytes!("../../../assets/app.ico"),
        },
    ];
    const PROCESSES: &'static [&'static str] = &[APP_EXE_NAME];

    fn texts(lang: Language) -> Texts {
        match lang {
            Language::En => Texts {
                description: "Shows whether Caps Lock is on or off with an icon in the notification area.\n\nIt starts automatically when you sign in to Windows. No settings, no network access.".into(),
                usage: "caps-indicator runs in the notification area and starts automatically when you sign in.\n\nTo stop it from starting with Windows, untick Start with Windows in its tray icon menu.".into(),
                uninstall_question: "The program, its Start Menu shortcut and its startup entry will be removed.".into(),
                uninstall_done: "The application, its shortcut and its startup entry have been removed.".into(),
                arch_reason: None,
            },
            Language::Es => Texts {
                description: "Muestra si Bloq Mayús está activado o desactivado con un icono en el área de notificación.\n\nSe inicia automáticamente al entrar en Windows. Sin configuración ni acceso a la red.".into(),
                usage: "caps-indicator se ejecuta en el área de notificación y se inicia automáticamente al entrar en Windows.\n\nPara que no se inicie con Windows, desmarca Iniciar con Windows en el menú de su icono de notificación.".into(),
                uninstall_question: "Se eliminarán el programa, su acceso directo del menú Inicio y su inicio automático.".into(),
                uninstall_done: "Se han eliminado la aplicación, su acceso directo y su inicio automático.".into(),
                arch_reason: None,
            },
        }
    }

    fn install(paths: &Paths, _lang: Language) -> Result<(), String> {
        // On by default for a fresh install only: updates and repairs keep the
        // user's choice, which they can change from the tray menu.
        let uninstall_key = format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
            Self::ID
        );
        let already_installed =
            get_reg_string(HKEY_CURRENT_USER, &uninstall_key, "DisplayVersion").is_ok();
        if already_installed {
            return Ok(());
        }
        let command = format!("\"{}\"", paths.install_dir.join(APP_EXE_NAME).display());
        set_reg_string(HKEY_CURRENT_USER, RUN_REG_KEY, Some(Self::ID), &command)
            .map_err(|code| format!("Cannot add caps-indicator to startup (error {code})"))
    }

    fn uninstall(_paths: &Paths) {
        let _ = delete_reg_value(HKEY_CURRENT_USER, RUN_REG_KEY, Self::ID);
    }

    fn shortcuts(paths: &Paths, lang: Language) -> Vec<Shortcut> {
        vec![Shortcut {
            name: Self::DISPLAY_NAME.into(),
            target: paths.install_dir.join(APP_EXE_NAME),
            description: lang
                .pick(
                    "Caps Lock indicator for the notification area",
                    "Indicador de Bloq Mayús para el área de notificación",
                )
                .into(),
        }]
    }

    fn launch(paths: &Paths) {
        let _ = Command::new(paths.install_dir.join(APP_EXE_NAME)).spawn();
    }
}

fn main() {
    setup::run::<CapsIndicator>();
}
