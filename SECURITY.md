# Security Policy

## Reporting a vulnerability

Please report security issues privately through **GitHub Security Advisories** ("Report a vulnerability" in the *Security* tab of this repository) instead of opening a public issue. You will receive an answer as soon as possible.

## Supported versions

Only the latest release receives fixes.

## Privacy and security properties

- **No keystroke capture**: caps-indicator reads only the Caps Lock toggle state with `GetKeyState(VK_CAPITAL)`. It installs no keyboard hook and never sees which keys are pressed.
- **No network access**: the program contains no networking code and sends no telemetry.
- **Nothing stored**: the application writes no files; its only registry write is the per-user startup entry, which you can toggle from the tray menu. The installer only adds its program folder, a Start Menu shortcut, the startup entry and the *Settings → Apps* entry, all for the current user, and the uninstaller removes them.
- **Standard user rights**: no administrator privileges, drivers or services are needed.
