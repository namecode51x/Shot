# NameShot

A local-first, lightweight screenshot utility for Windows 10/11 (x64). Press one hotkey,
drag a region, annotate it right on top of the selection — inside the same overlay, no
second window — then copy it to the clipboard or save it with
the native Windows file dialog — no OS snipping overlay, no accounts, nothing leaves your
machine.

- **No network code at all**: no sockets, no HTTP/DNS APIs, no updater, no telemetry
  (verified by source search; the only dependency is the `windows` crate).
- **Native Windows app**: tray icon, global hotkey, dark title bars, standard
  `IFileSaveDialog`, settings in `%APPDATA%\NameShot\settings.ini`.
- **Modern dark interface**: every surface is driven by one token set
  (`src/theme.rs`, design in [`docs/UI_REFRESH_SPEC.md`](docs/UI_REFRESH_SPEC.md)) —
  near-black palette, a single cool-blue accent, icon-only tool rail with chip
  tooltips, a live size chip while selecting, grouped Settings rows.

## Status

Built and verified in phases on a real Windows 10 x64 machine — full evidence in
[`docs/BUILD_LOG.md`](docs/BUILD_LOG.md), design in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

| Executed phase (BUILD_LOG) | Scope | State |
|---|---|---|
| 0 | Repository, toolchain, architecture | done |
| 1 | OS-free core, 46 unit tests | done |
| 2 | Windows layer + full UI, 0 warnings | done |
| 3 | Live GUI verification (steps A–K), 50 tests | done |
| 4 | Icon embedding (`build.rs`), README | done |
| 4.5 | Redaction-correctness tests (original plan Phase 4, item 2), 54 tests | done |
| 5 | Settings recovery, accessibility names, release measurements, no-network audit (original plan Phase 5), 26/26 live checks | done |
| 6 | Verification gates, production-path review, live redaction pass, packaging (original plan Phase 6), 31/31 live checks | done |
| A–E (auto) | Corrective task: in-overlay editing (no separate window), flicker-free painting, hover hints; automated gates | done (live re-run deferred by request) |
| UI | Modern-dark visual refresh: design tokens, overlay chrome, Settings/About ([`docs/UI_REFRESH_SPEC.md`](docs/UI_REFRESH_SPEC.md)) | code complete, gates green; live visual QA + re-packaging pending review |

**Authoritative plan:** the original *Phased execution plan* (Phases 0–6) from the
project's first requirement message; **all six phases are complete**. The executed-phase
numbering above is the chronological build record; the mapping, the final acceptance
checklist with per-box evidence, and the few criteria still open are listed in
[`docs/BUILD_LOG.md`](docs/BUILD_LOG.md) ("Plan reconciliation").

Only behaviour that was actually exercised is claimed; see the build log for what was and
wasn't verified (e.g. bare `PrintScreen` registration conflicts with explorer's snipping
hook on this machine — fallbacks: tray menu capture or `Alt+PrintScreen` in Settings;
see "Hotkey note" below).

## Features

- **Capture**: global hotkey (default `PrintScreen`, rebindable) → full-screen overlay →
  rubber-band selection (arrow keys nudge) → releasing the mouse starts editing
  **in the same overlay** (`Enter` still works as the keyboard path) — the frozen
  selection stays on screen, no separate editor window.
- **Editing tools**: pen, line, arrow, filled rectangle, outlined box, text, highlight,
  mosaic (pixelate) and solid redaction; colour/width strip, undo/redo; hovering a
  button shows its hint (with keyboard shortcut) in the status chip. `Ctrl+C` copies
  and `Ctrl+S` saves — once the action really succeeded (image on the clipboard,
  bytes on disk) the session **closes itself** and a tray balloon confirms; a failed
  copy, a cancelled Save-As dialog or a write error keeps the session open and is
  reported in the status chip. The Close button, `Esc` and the window close command
  discard the
  session immediately — there is **no confirmation dialog**. A text box keeps its
  draft (and its contents) while focus moves to another window or to another
  NameShot control — nothing is committed or discarded by losing focus; only an
  explicit action places the text (`Enter`, or a copy/save command), while `Esc` in
  the box cancels only the draft — with the box gone, the next `Esc` closes the
  session.
- **Output**: copy as `CF_DIB` (pastes anywhere), save via native dialog (PNG/JPEG with
  quality setting), optional *Quick save* tray action that never overwrites an existing file.
- **Tray menu**: Capture, Quick save (optional), Settings, About, Exit. Single instance —
  launching twice just points at the running copy.
- **Settings**: hotkey with conflict detection, output format, JPEG quality, save folder,
  quick-save toggle, start-at-sign-in (one `HKCU\...\Run` value, off by default).
  Default save folder: Pictures.
- **Diagnostics**: a panic writes `%TEMP%\nameshot_panic.txt` instead of dying silently.

## Running

- **Release package:** `dist\NameShot-0.1.0-win-x64.zip` — extract anywhere and run
  `nameshot.exe`. No installer, no admin rights, no runtime prerequisites: the exe
  imports only Windows system DLLs and the inbox Universal CRT (checked with
  `objdump -p`) — **no WebView2, no .NET, no VC++ redistributable, nothing bundled**.
  The zip also carries `README.md` and `docs/` (ARCHITECTURE.md, BUILD_LOG.md).
- **From source:** `cargo build --release`, run `target\release\nameshot.exe`.
- First run: tray icon appears (no window), default hotkey `PrintScreen` if free
  (otherwise the documented fallback), settings are created at
  `%APPDATA%\NameShot\settings.ini`. Uninstall = delete the exe (optionally also
  `%APPDATA%\NameShot`).
- Not code-signed — SmartScreen may warn on first launch on another machine.

## Building

Requirements:

- Windows 10/11 x64.
- Rust stable (verified: `rustc 1.99.0`, host `x86_64-pc-windows-gnu`).
- A MinGW-w64 toolchain providing `gcc` **and** `windres` on `PATH`
  (verified: winlibs gcc 16.2.0 in `C:\tools\mingw64\bin`).
  MSVC + Windows SDK also works: `build.rs` falls back to `rc.exe`.

```powershell
$env:PATH = "C:\tools\mingw64\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build                 # → target\debug\nameshot.exe
cargo build --release       # → target\release\nameshot.exe (verified: exit 0)
cargo test                  # 114 tests: 104 unit + 10 integration, 0 failures
cargo check --all-targets   # 0 errors, 0 warnings
```

The crate targets Windows only. The build script itself is portable: on non-Windows
targets it returns immediately (`CARGO_CFG_TARGET_OS != "windows"`), so cross-platform
`cargo check` of the build logic never touches `windres`/`rc.exe`.

### Application icon

- `assets/icon.png` is the project artwork (2508×2508 RGBA, read-only).
  `assets/icon.ico` is the **final multi-resolution icon** generated from it
  by `assets/make_icon.ps1` — 7 PNG-compressed 32-bit frames
  (16/24/32/48/64/128/256 px, square, transparency preserved 1:1) — embedded
  into the executable by `build.rs` (`1 ICON`, RT_ICON ids 1–7 +
  RT_GROUP_ICON 1).
- The running app loads that embedded resource: window classes
  (`hIcon`/`hIconSm` in `register_class` → title bars, Alt-Tab, taskbar)
  and the tray icon. `tests/icon_asset.rs` locks the `.ico` structure
  (frame count/sizes/bpp, PNG frames, RGBA, contiguous layout).
- To change the icon: replace `assets/icon.png`, run
  `powershell -ExecutionPolicy Bypass -File assets\make_icon.ps1`, then
  `cargo build` — no code changes; the build script re-runs automatically
  (`cargo:rerun-if-changed=assets/icon.ico`).
- `assets/make_placeholder_icon.ps1` is the legacy *temporary placeholder*
  generator, kept for reference but **superseded** — do not run it over the
  final `assets/icon.ico`.
- Without `windres`/`rc.exe` (or if `assets/icon.ico` is missing) the build
  still succeeds, prints one `cargo:warning`, and the exe keeps the default
  icon.

## Hotkey note

Windows itself may already hold bare `PrintScreen` (explorer's snipping hook), in which
case registration fails with *hotkey unavailable*; NameShot stays running — capture from
the tray menu or choose a modified hotkey (e.g. `Alt+PrintScreen`) in Settings.

## Measured resources

Release exe; Windows 10 IoT Enterprise LTSC build 19044, 1920×1200 single monitor,
interactive session; numbers from external OS process counters (no in-app benchmark);
idle after 8 s warm-up over a 20 s window. Full conditions and methodology:
[`docs/BUILD_LOG.md`](docs/BUILD_LOG.md) (Phase 5).

| State | Private working set | Peak working set | CPU | TCP/UDP sockets |
|---|---|---|---|---|
| idle (tray) | 1.6 MB | 8 MB | 0.000 s / 20 s (~0% of a core) | 0 / 0 |
| capture session (overlay) | 19.7 MB | 37.2 MB | short session | 0 / 0 |
| editor session | 7.9 MB | 37.2 MB | short session | 0 / 0 |

## Manual QA checklist

Concise pass/fail checklist for a clean machine (full evidence of every point:
[`docs/BUILD_LOG.md`](docs/BUILD_LOG.md)):

- [ ] Launch → tray icon appears, no main window; launching a second instance points at the running copy.
- [ ] Default hotkey opens the full-screen overlay; `Esc` cancels; drag a region and release the mouse → edits in place (same window, no Enter needed, `Enter` still works, no flicker on tool clicks).
- [ ] Every tool visibly draws (pen, line, arrow, rect, box, text, highlight, mosaic, solid); `Ctrl+Z`/`Ctrl+Y` undo/redo; hovering a button shows its hint in the status chip.
- [ ] `Ctrl+C` copies **and closes the session** (tray balloon confirms; no dialog, no focus steal); `Ctrl+S` writes the file **and then closes**; a cancelled Save-As or any failure keeps the session open and is reported in the status chip; `Esc`/Close discard immediately, never asking to confirm.
- [ ] Redacted regions in the saved file are flattened — mosaic cells/solid blocks contain no original pixels.
- [ ] Save uses the native dialog (PNG/JPEG/BMP); *Quick save* never overwrites an existing file.
- [ ] Change hotkey/format/folder in Settings → persists across restart; corrupt `settings.ini` recovers to safe defaults.
- [ ] Hotkey conflict (e.g. explorer holding `PrintScreen`) shows a clear message; app stays usable via tray.
- [ ] Idle: 0 CPU over 20 s, 0 sockets; exit from tray leaves no process, no stray files, no panic log.
- [ ] (Not verified on this build machine) multi-monitor offsets, mixed-DPI desktops, DRM/secure-desktop captures.

## Known limitations

- Verified on **one machine**: Windows 10 x64, single 1920×1200 monitor. Multi-monitor
  and mixed-DPI behaviour is implemented per design (virtual-screen capture,
  `PER_MONITOR_AWARE_V2`) but has not been live-tested — recorded as an open criterion
  in the build log.
- GDI capture cannot read DRM/protected surfaces or the secure desktop (UAC); those
  regions may appear black.
- Not code-signed; no installer; SmartScreen may warn on first run elsewhere.
- Mosaic is pixelation for readability, **not** an anonymity guarantee against
  identification (shape, context, or small-region attacks).
- The in-overlay editor has no zoom/pan; the whole selection is shown fitted.
  (Zoom/pan still exists in the code's separate editor window, which the current
  flow never opens.)

## Troubleshooting

- **"Hotkey unavailable"** — another app (commonly explorer's snipping hook) owns
  `PrintScreen`. Use tray ▸ Capture, or pick e.g. `Alt+PrintScreen` in Settings.
- **Crash instead of silence** — a panic writes `%TEMP%\nameshot_panic.txt`
  (backtrace included); the app does not die silently.
- **Paste shows nothing** — NameShot copies `CF_DIB` + `PNG`; the target app must accept
  one of those (any Office/Paint/editor does).
- **Start-at-login not applying** — it is off by default and only written when enabled
  in Settings (`HKCU\...\Run`).

## Project layout

```
build.rs                  embeds assets/icon.ico into the exe (Windows only)
assets/                   icon.png (artwork, source of truth), icon.ico (final
                          7-frame icon), make_icon.ps1 (generator)
src/
  lib.rs, main.rs         crate root (GUI subsystem, panic hook → %TEMP%\nameshot_panic.txt)
  image/ annotate/ flatten OS-free core: pixels, shapes, rasterisation
  config/ hotkey/ output/ strings/ error   OS-free core: settings, hotkeys, filenames, text
  theme.rs                design tokens (palette, type, spacing, icons) for the whole UI
  win/                    Win32 layer: gdi, dark, tray, hotkey, clipboard, dialogs, a11y
  capture.rs encode.rs    GDI capture, WIC encode/decode
  app.rs overlay.rs os.rs OS gate + DPI, message loop, tray dispatch, capture overlay
  editor/ settings_ui.rs  annotation tools (shared with the inline overlay editor; the
                          separate editor window exists but the flow no longer opens it),
                          Settings/About windows
tests/encode_roundtrip.rs WIC encode/decode round-trip integration tests
tests/icon_asset.rs        assets/icon.ico structure contract (7 frames, PNG RGBA)
tests/redaction.rs        redaction-correctness integration tests (ARCHITECTURE.md model)
docs/                     ARCHITECTURE.md, BUILD_LOG.md, UI_REFRESH_SPEC.md
dist/                     release package (zip: exe + README + docs)
```

## License

All rights reserved (personal project; no license granted yet).
#   N a m e S h o t  
 #   N a m e S h o t  
 #   N a m e S h o t  
 #   N a m e S h o t  
 