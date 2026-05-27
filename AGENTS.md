# CleanKakao

## Project Overview
KakaoTalk PC ad-blocking system tray app for Windows.

## Architecture
Walks the KakaoTalk window hierarchy via the Win32 API, identifies ad child windows, hides them with `ShowWindow(SW_HIDE)`, and reclaims the freed space by repositioning the content area with `SetWindowPos`. Runs entirely in the tray; the settings window is built with iced.

## Data Flow
1. On launch, load config from `%AppData%\cleankakao\config.toml` (fall back to defaults if missing).
2. `process_watcher` periodically polls for the `KakaoTalk.exe` process.
3. When KakaoTalk is detected, `adblocker` scans the window hierarchy and hides ad surfaces.
4. When the user opens the settings window from the tray menu, render the iced `SettingsView`.
5. When settings change, persist the config to disk and push the updates to `adblocker` live.

## Tech Stack
- **Language**: Rust 1.95
- **Win32 API**: `windows` crate v0.62 — `FindWindowExW`, `EnumChildWindows`, `ShowWindow`, `SetWindowPos`, `CreateToolhelp32Snapshot`.
- **System tray**: `tray-icon` v0.21 + `muda` v0.17 (context menu).
- **Settings window UI**: `iced` v0.13 with a custom WinUI3 theme.
- **Settings storage**: `serde` v1 + `toml` v0.8.
- **Automatic updates**: `reqwest` v0.12 against the GitHub Releases API.
- **Build / deployment**: `cargo` + GitHub Actions + GitHub Releases.

## Project Layout
- `src/main.rs` — entry point, tray + event loop, `#![windows_subsystem = "windows"]` so release builds have no console.
- `src/adblocker.rs` — ad-window detection and hide/resize logic.
- `src/process_watcher.rs` — KakaoTalk process discovery and lifecycle tracking.
- `src/config.rs` — TOML config load/save (defaults + user overrides).
- `src/tray.rs` — `tray-icon` + `muda` wiring, tray events.
- `src/ui/` — iced settings window.
  - `settings.rs` — window state + `Message` enum.
  - `theme.rs` — custom WinUI3-style theme.
- `src/updater.rs` — release manifest check + self-update via `reqwest`.
- `assets/` — `icon_active.ico`, `icon_inactive.ico`, `FluentSystemIcons-Regular.ttf`. Not committed; see `assets/README.md`.

## Build & Run
- Dev: `cargo run`
- Release: `cargo build --release` — produces a size-optimized binary (`opt-level = "z"`, LTO, strip, single codegen unit).
- Logging: set `RUST_LOG=cleankakao=debug` to see `log` crate output.

## Coding Conventions
- Rust 2024 edition.
- `snake_case` for items, modules, and files.
- Comments are written in English.
- Unsafe blocks are limited to Win32 API calls.
- Every `unsafe` block carries a `// SAFETY:` comment explaining why the invariants hold.

## KakaoTalk Window Structure
- Main window title: "카카오톡" / "KakaoTalk" / "カカオトーク" (match all three for locale coverage).
- Child window classes: `EVA_ChildWindow`, `EVA_Window`.
- Ad-area handling principle: hide the ad window with `ShowWindow(SW_HIDE)`, then expand the chat content area with `SetWindowPos`.

## Icon & UI
- Tray icon switches between `icon_active.ico` and `icon_inactive.ico` based on KakaoTalk running state and blocking toggle.
- Settings window list icons render via Fluent UI System Icons, loaded as an iced font so we can draw the Unicode glyphs directly.

## Pitfalls & Safeguards
- **False positives**: do not hide non-ad windows (birthday greetings, polls, settings dialogs). Require PID verification plus an allow/deny class-name list.
- **Cross-process isolation**: never touch windows that do not belong to the KakaoTalk process. Verify with both `GetWindowThreadProcessId` and `QueryFullProcessImageNameW`.
- **CPU usage**: adaptive polling — 3–5 s in steady state, pause while the main window is minimized.
- **Startup timing**: after detecting the KakaoTalk PID, wait for the HWND to appear and allow an initialization delay before scanning.
- **Persistent banner frames**: ad content window and its outer frame window must be handled as a pair; hiding only one leaves a ghost.
- **Z-order side effect**: always pass `SWP_NOZORDER | SWP_NOACTIVATE` to `SetWindowPos`. Omitting these pins the main window above the chat window.
- **Restart detection**: when KakaoTalk is closed and relaunched, clear the cached HWND and any per-process state so the new PID is picked up cleanly.
- **Media protection**: video playback windows can resemble ads. Keep media-related class/title keywords on the whitelist so playback is never interrupted.
