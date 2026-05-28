# Assets

The release build embeds these assets at compile time, so they are part of the
repository:

- `icon_active.ico` — tray icon shown when blocking is enabled. Final design TBD; use any 256x256 ICO as a temporary stand-in during development.
- `icon_inactive.ico` — tray icon shown when blocking is disabled. Same temporary-icon note as above.
- `fonts/FluentSystemIcons-Regular.ttf` — Fluent UI System Icons (Regular), used for settings-window list glyphs.
- `fonts/PretendardJP-Medium.otf` — settings-window body font.
- `fonts/PretendardJP-SemiBold.otf` — settings-window heading font.

## Refreshing the Fluent icon font

Run from the repo root:

```powershell
powershell -ExecutionPolicy Bypass -File assets\fetch.ps1
```

The script downloads `FluentSystemIcons-Regular.ttf` from `microsoft/fluentui-system-icons` pinned to a specific commit (see `assets/fetch.ps1`). It is a no-op if the file is already present.
