# CleanKakao

## Project Overview
KakaoTalk PC ad-blocking system tray app for Windows.

## Architecture
Walks the KakaoTalk window hierarchy via the Win32 API, identifies ad child windows, hides them with `ShowWindow(SW_HIDE)`, and reclaims the freed space by repositioning the content area with `SetWindowPos`. The host app lives in the tray; the iced settings window runs as a separate `--settings` subprocess. A WinEvent hook watches KakaoTalk window show/create/move/restore events so ad blocking is reapplied immediately after restore.

## Data Flow
1. On launch, load config from `%LocalAppData%\cleankakao\config.toml` (fall back to defaults if missing).
2. `process_watcher` periodically polls for the `KakaoTalk.exe` process.
3. When KakaoTalk is detected and its main HWND is ready, `adblocker` scans the window hierarchy and hides ad surfaces.
4. `window_events` installs out-of-context WinEvent hooks for the KakaoTalk PID and triggers short reapply bursts after foreground, restore, create/show, and location-change events.
5. A periodic fallback worker reapplies blocking while KakaoTalk is running.
6. When the user opens settings from the tray menu, spawn the `--settings` subprocess and render the iced settings window there.
7. Settings changes are saved to disk; the tray process polls the config file, refreshes shared config, updates the tray icon/check state, and calls `apply_all` or `restore_all` live.

## Tech Stack
- **Language**: Rust 1.95
- **Win32 API**: `windows` crate v0.62 — `FindWindowW`, `EnumWindows`, `EnumChildWindows`, `EnumThreadWindows`, `ShowWindow`, `SetWindowPos`, `SetWinEventHook`, `CreateToolhelp32Snapshot`.
- **System tray**: `tray-icon` v0.21 + `muda` v0.17 (context menu).
- **Settings window UI**: `iced` v0.13 with a custom WinUI3 theme.
- **Settings storage**: `serde` v1 + `toml` v0.8.
- **Automatic updates**: `reqwest` v0.12 against the GitHub Releases API. Current behavior checks the latest release, shows a Windows toast notification, and opens the GitHub Releases page from the toast; it does not self-replace the executable.
- **Build / deployment**: `cargo` + `winres` for Windows executable metadata + GitHub Actions tag releases.

## Project Layout
- `src/main.rs` — entry point, tray + event loop, `#![windows_subsystem = "windows"]` so release builds have no console.
- `src/adblocker.rs` — ad-window detection and hide/resize logic.
- `src/process_watcher.rs` — KakaoTalk process discovery and lifecycle tracking.
- `src/window_events.rs` — WinEvent hook worker for restore/show/create/location-change reapply triggers.
- `src/config.rs` — TOML config load/save (defaults + user overrides).
- `src/tray.rs` — `tray-icon` + `muda` wiring, tray events.
- `src/ico.rs` — picks the best-fit image from embedded `.ico` bytes for the tray and window/taskbar icons.
- `src/win32.rs` — shared Win32 helpers (`class_name`, `window_text`).
- `src/constants.rs` — shared constants (e.g. `KAKAOTALK_EXE`).
- `src/ui/` — iced settings window.
  - `settings.rs` — window state + `Message` enum.
  - `theme.rs` — custom WinUI3-style theme.
- `src/updater.rs` — GitHub Releases update check, periodic update worker, and Windows toast notification.
- `build.rs` — embeds Windows executable resources such as icon and version metadata.
- `.github/workflows/release.yml` — tag-triggered Windows release build and GitHub Release asset upload.
- `assets/` — build-time embedded icons and fonts: `icon_active.ico`, `icon_inactive.ico`, `fonts/FluentSystemIcons-Regular.ttf`, `fonts/PretendardJP-Medium.otf`, and `fonts/PretendardJP-SemiBold.otf`.

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
- Relevant classes: `EVA_ChildWindow`, `EVA_Window`, `EVA_Window_Dblclk`, Chrome render/widget surfaces, and `Chrome Legacy Window`.
- Ad-area handling principle: hide ad windows with `ShowWindow(SW_HIDE)`, then expand known main content panes with `SetWindowPos`.

## Icon & UI
- Tray icon switches between `icon_active.ico` and `icon_inactive.ico` based on the blocking toggle.
- Settings window icons render via Fluent UI System Icons, and text uses Pretendard JP fonts loaded as iced fonts.

## Pitfalls & Safeguards
- **False positives**: do not hide non-ad windows (birthday greetings, polls, settings dialogs). Require PID/image verification plus strict class, parent, descendant, and geometry checks.
- **Cross-process isolation**: never touch windows that do not belong to the KakaoTalk process. Verify with both `GetWindowThreadProcessId` and `QueryFullProcessImageNameW`.
- **CPU usage**: process polling and periodic fallback are intentionally modest, while WinEvent hooks handle restore/show/create/move bursts. `apply_all` skips while the main window is minimized.
- **Startup timing**: after detecting the KakaoTalk PID, wait for the HWND to appear, then reapply repeatedly during a short startup window so late-created ad surfaces are caught.
- **Persistent banner frames**: ad content window and its outer frame window must be handled as a pair; hiding only one leaves a ghost.
- **Z-order side effect**: always pass `SWP_NOZORDER | SWP_NOACTIVATE` to `SetWindowPos`. Omitting these pins the main window above the chat window.
- **Restart detection**: when KakaoTalk is closed and relaunched, clear the cached HWND and any per-process state so the new PID is picked up cleanly.
- **Media protection**: video playback windows can resemble ads. Prefer class, parent, PID/image, and geometry checks over title-text keyword rules.
