# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-10-14

First public release.

### Added
- Caps Lock indicator in the notification area: lowercase `a` when off, uppercase `A` when on, with a matching tooltip.
- Works while elevated apps, games or Remote Desktop have the focus; no keyboard hook, keystrokes are never captured.
- Survives Explorer restarts and keeps retrying until the notification area is available at sign-in.
- Per-monitor DPI awareness with multi-resolution icons (16–256 px).
- Single instance: starting it again says that it is already running.
- English and Spanish, following the Windows display language.
- Per-user installer (`caps-indicator-setup.exe`) for x64, x86 and ARM64: Start Menu shortcut, start with Windows on a fresh install (the app launches right after installing), *Settings → Apps* entry, update/repair, unattended mode and clean uninstall.
- "Start with Windows" toggle in the tray icon menu; updates and repairs keep the user's choice.
