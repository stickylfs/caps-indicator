//! Embeds the application files, built beforehand for the same target and
//! profile, into the installer.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Files embedded in the installer.
const PAYLOAD: &[&str] = &["caps-indicator.exe"];
/// Packages that build [`PAYLOAD`].
const PAYLOAD_PACKAGES: &str = "caps-indicator";

fn main() {
    // Common Controls v6: TaskDialog and modern control visuals.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../../target"));

    // `cargo build --target <triple>` writes to target/<triple>/<profile>; a
    // build without `--target` (host only) writes to target/<profile>.
    let mut search_dirs = vec![target_dir.join(&target).join(&profile)];
    if target == host {
        search_dirs.push(target_dir.join(&profile));
    }

    for name in PAYLOAD {
        let destination = out_dir.join(name);
        for dir in &search_dirs {
            println!("cargo:rerun-if-changed={}", dir.join(name).display());
        }
        match search_dirs
            .iter()
            .map(|d| d.join(name))
            .find(|p| p.exists())
        {
            Some(source) => {
                fs::copy(&source, &destination).expect("cannot copy the installer payload");
            }
            None => {
                // Not built yet (e.g. `cargo check` or `cargo test`): embed an
                // empty placeholder. The installer refuses to run without payload.
                println!(
                    "cargo:warning={name} not found: build {PAYLOAD_PACKAGES} for {target} \
                     ({profile}) first, or the installer will have no payload"
                );
                fs::write(&destination, b"").expect("cannot write the payload placeholder");
            }
        }
    }

    println!("cargo:rerun-if-changed=../../assets/app.ico");
    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/app.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to compile Windows resource: {e}");
        }
    }
}
