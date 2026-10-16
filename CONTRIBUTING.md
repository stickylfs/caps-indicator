# Contributing to caps-indicator

Thank you for your interest in contributing! Bug reports, fixes, translations and documentation improvements are all welcome.

## Development setup

### Prerequisites

- Windows 10 or 11
- [Rust](https://rustup.rs/) 1.82 or newer
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the *Desktop development with C++* workload (MSVC linker and Windows SDK)

### Build and test

```bash
git clone https://github.com/<your-username>/caps-indicator.git
cd caps-indicator

cargo test --workspace            # run all test suites
cargo run -p caps-indicator       # run the tray app (debug build)

# Release installer: it embeds the application, so build it first.
cargo build --release -p caps-indicator
cargo build --release -p caps-indicator-installer
```

The installer is written to `target/release/caps-indicator-setup.exe`. Building the installer before the application prints a warning and produces an installer that refuses to run. For other architectures add `--target i686-pc-windows-msvc` or `--target aarch64-pc-windows-msvc` to both commands.

## Project layout

- `crates/caps-indicator/src/main.rs`: the whole application (Win32 message loop, tray icon, Caps Lock polling, localized strings).
- `crates/caps-indicator-installer`: per-user installer and uninstaller. `setup.rs` holds the app-independent install flow; `main.rs` describes the app (files, registrations and texts).
- `assets/caps-on.ico`, `assets/caps-off.ico`: tray icons, embedded with `include_bytes!`. Replacement icons should contain 16, 32, 48 and 256 px layers.
- `assets/app.ico`: executable and installer icon.

## Pull request checklist

1. Format the code: `cargo fmt --all`
2. No lint warnings: `cargo clippy --workspace --all-targets -- -D warnings`
3. Tests pass: `cargo test --workspace`
4. Update `CHANGELOG.md` for user-visible changes.
5. Describe what the change does and why in the pull request.

## Code style

- Keep the dependency footprint minimal: the application depends on `windows-sys` only.
- Wrap Win32 calls in small safe helpers where possible and document why each `unsafe` block is sound when it is not obvious.
- User-facing text must be available in English and Spanish.

## Releasing

1. Update the version in the workspace `Cargo.toml` and add a section to `CHANGELOG.md`.
2. Tag the commit (`git tag v1.2.3 && git push --tags`).
3. The **Release** workflow builds the x64, x86 and ARM64 installers, generates `SHA256SUMS.txt` and publishes a GitHub release.

## License

By contributing, you agree that your contributions are licensed under the MIT License, like the rest of the project.
