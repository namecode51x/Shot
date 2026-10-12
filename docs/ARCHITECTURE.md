# NameShot — Architecture

NameShot is a native Windows screenshot utility written in stable Rust. It has no browser
runtime, no web view, no server, and no network dependency for any core feature.

Target: **Windows 10/11, x64** (build machine: Windows 10 IoT Enterprise LTSC, build 19044).

## High-level data flow

```
hotkey / tray "Capture"
        │
        ▼
 one-shot GDI capture of the virtual screen ──► Frame (BGRA, physical px, virtual bounds)
        │                                                │
        ▼                                                │ (released when session ends)
 selection overlay (borderless, topmost, spans virtual desktop, negative coords OK)
        │  drag / move / resize / full-screen / restart / Esc
        ▼
 cropped selection ──► editor window (dark, compact, native scale + zoom/pan)
        │
        │  annotations (pen/line/arrow/rect/box/text/highlight)  +  redactions (mosaic/solid)
        ▼
 flatten()  ► final RGBA raster (only this leaves the app)
        ├──► clipboard (CF_DIB + "PNG" format)   [native Win32 clipboard]
        └──► file (WIC encoder: PNG / JPEG / BMP) [native Save As dialog]
```

## Module boundaries

| Module | Responsibility | Key Win32 APIs |
|---|---|---|
| `main.rs` | entry point (`windows_subsystem = "windows"`), panic hook writing `%TEMP%\nameshot_panic.txt`, hands off to `run()` | — (std only) |
| `app.rs` | `run()`: supported-OS gate, DPI init, STA COM init, hidden app window, blocking message loop, `AppState` (settings + session state), tray menu dispatch, session orchestration | `CoInitializeEx`, `GetMessageW`, `Shell_NotifyIconW`, `CreatePopupMenu`, `TrackPopupMenu` |
| `strings.rs` | all user-visible strings, one table, localization-ready | — |
| `error.rs` | typed `Error` + `Result`, human-readable messages (`FormatMessageW` for Win32 codes) | `FormatMessageW` |
| `config.rs` | versioned settings file (minimal INI), parse/serialize, safe defaults, validation | `SHGetKnownFolderPath`/`FOLDERID_RoamingAppData` |
| `hotkey.rs` | hotkey string parse/print, VK/modifier constants (Win32 numeric ABI values, no calls) | — (pure Rust) |
| `output.rs` | default filename generation (format + timestamp + no-overwrite suffix logic) | — (pure Rust) |
| `os.rs` | supported-OS check, DPI helper, monitor enumeration, virtual-screen metrics | `RtlGetVersion`, `SetProcessDpiAwarenessContext`, `GetSystemMetrics`, `EnumDisplayMonitors` |
| `win/tray.rs` | tray icon lifecycle, menu, balloon messages | `Shell_NotifyIconW`, `LoadImageW` |
| `win/hotkey.rs` | `RegisterHotKey`/`UnregisterHotKey`, conflict detection (error 1409 → non-fatal) | `RegisterHotKey` |
| `win/a11y.rs` | exposes `PROPID_ACC_NAME` accessibility names on Settings/editor controls (best-effort) | `IAccPropServices::SetHwndPropStr` |
| `win/gdi.rs` | RAII guards for `HDC`/`HBITMAP`/`HFONT`/`HBRUSH`/`HPEN`/`HGDIOBJ`, DIB section helpers | `CreateCompatibleDC`, `CreateDIBSection`, `DeleteObject` |
| `win/clipboard.rs` | open/empty/set `CF_DIB` + registered `PNG` format, correct `GlobalAlloc` ownership | `OpenClipboard`, `SetClipboardData` |
| `win/dialog.rs` | native Save As (`IFileSaveDialog`), message boxes, browse folder | `IFileSaveDialog`, `CoCreateInstance` |
| `win/dark.rs` | dark title bars / control theming | `DwmSetWindowAttribute`, `SetWindowTheme` |
| `capture.rs` | one-shot virtual-screen capture into a top-down 32-bit DIB | `GetDC(NULL)`, `BitBlt`, `CreateDIBSection` |
| `overlay.rs` | borderless topmost selection overlay: dim, drag-select, 8 handles, move, full-screen, Esc, restart | `CreateWindowExW`, `SetCapture`, `InvalidateRect` |
| `editor/mod.rs` | editor window, toolbar (owner-draw buttons), input, zoom/pan, keyboard shortcuts, focus | `CreateWindowExW`, `SetFocus`, `WM_MOUSEWHEEL` |
| `editor/toolbar.rs` | toolbar layout, buttons, color/stroke swatches, focus ring | owner-draw `BS_OWNERDRAW` |
| `settings_ui.rs` | Settings and About windows (hotkey/format/quality/folder/quick-save/startup controls) | `CreateWindowExW`, `IFileSaveDialog` (folder browse) |
| `annotate.rs` | annotation model (pure data), undo/redo stacks — no OS code, fully testable | — |
| `flatten.rs` | ordered rasterization of annotations + redactions into a new buffer; cached prefix for interactive redraw | — (pure Rust) |
| `image/mod.rs` | `Frame` (BGRA8 buffer), bounds-checked pixel access, crop, blend, line/rect/polyline raster | — (pure Rust) |
| `image/redact.rs` | mosaic (block average) and solid opaque redaction — pure, tested | — (pure Rust) |
| `text.rs` | text annotation rasterization (GDI `DrawTextW` into a DIB section wrapping the frame) | `CreateDIBSection`, `DrawTextW` |
| `encode.rs` | PNG/JPEG/BMP encoding of the flattened frame via WIC, byte-exact header checks | `IWICImagingFactory`, `IWICBitmapEncoder` |

### Boundary rules

- `image/`, `annotate.rs`, `flatten.rs`, `config.rs`, `strings.rs` contain **no Win32 calls**
  (except `text.rs` for text raster, explicitly marked) so they can be unit-tested directly.
- `win/*` modules own handles/ownership; every GDI object, DC, clipboard session and hotkey is
  released via RAII guards on success, cancel and error paths.
- Only `app.rs` decides which session is active: `Idle → Selecting → Editing → Idle`.
- Capture buffers live inside the active session struct; dropping the session frees them.

## Native API choices and rationale

| Concern | Choice | Why |
|---|---|---|
| Screen capture | **GDI `BitBlt` from `GetDC(NULL)`** into a top-down 32-bit DIB, one shot per session | Simplest reliable path; no runtime service, no D3D device, no per-frame polling; captures the whole virtual screen (incl. negative coordinates) in a single call. DXGI Desktop Duplication was evaluated and rejected for MVP: it needs a D3D11 device, per-output duplication interfaces, error-recovery loops (wait/bh types) — far more code for no benefit in a one-shot, local-only tool. Known limits are documented in README (DRM/protected surfaces, secure desktop). |
| Overlay | One borderless `WS_POPUP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW` window covering the virtual desktop, painting the frozen capture | No per-frame capture (no flicker, no stale frames), works across monitors, keeps input in one place. |
| DPI | `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` before any window; all coordinates in physical pixels | Mixed-DPI virtual desktops map 1:1 to captured pixels; no fractional coordinate drift. |
| Editor drawing | Flatten to an in-memory RGBA frame, present with one `BitBlt`/`StretchDIBits` per repaint | One code path shared by preview, clipboard and file export → what you see is what is exported. No continuous redraw; repaint only on invalidate. |
| Text rendering | GDI `DrawTextW` (ClearType) into a DIB section over the frame | Native quality, zero font crates. |
| Image encode | **WIC** (`IWICImagingFactory` PNG/JPEG/BMP) | Built into Windows; no third-party encoder crates, no extra runtime. |
| Clipboard | `CF_DIB` (+ registered `PNG` format) built manually from the flattened frame | Native formats, paste-tested; explicit buffer ownership. |
| Save dialog | `IFileSaveDialog` | Native, gives overwrite prompts for free (never silently overwrites). |
| Settings | Custom minimal INI in `%APPDATA%\NameShot\settings.ini`, versioned | ~100 lines, human readable, zero dependencies, unit-testable; never stores pixels. |
| Threading | Single UI thread + STA COM; message-only work via `GetMessage` (blocking) | Idle CPU ≈ 0: no timers, no polling, no background threads. |

## Dependencies

| Crate | Version | Why |
|---|---|---|
| `windows` | latest 0.x | The required, maintained Win32 binding crate. Only the specific `Win32_*` feature areas used are enabled. Everything else (raster, redaction, INI, strings, undo) is hand-written to keep the dependency graph minimal. |

No other runtime crates. No `image`, no `serde`, no `winres` (icon/version resources are emitted by a
tiny `build.rs` that calls MinGW `windres` when the GNU toolchain is used).

## Redaction correctness model

1. Annotations are data (`annotate.rs`), applied **in order** to a copy of the source frame.
2. A mosaic/solid region applied **after** another annotation also destroys that annotation's
   pixels (order in the list = order of application). Nothing else ever reads the source frame.
3. The only value that reaches the clipboard or a file is the flattened buffer; the source frame
   and the annotation list are dropped with the session. Encoders receive the flattened buffer
   only — there are no layers, no metadata side-cars, no hidden channels.
4. `tests/redaction.rs` proves this on synthetic images: mosaic output equals block averages and
   contains no original per-pixel values; solid output is fully opaque; flattened output differs
   from source only inside redaction regions; encode → decode round-trip equals the flattened
   buffer byte-for-byte. (Implemented and passing — 4 tests, delivered as BUILD_LOG
   Phase 4.5, original plan Phase 4 item 2.)

## Resource lifecycle

- Idle: hidden app window + tray icon + registered hotkey. `GetMessage` blocks → ~0% CPU.
- Capture session: one `Frame` (W×H×4 bytes for the virtual desktop) + overlay window.
- Editor session: cropped `Frame` + flatten cache (1 frame) + on-screen presentation bitmap.
- Everything is freed when the session struct is dropped (Esc, close, save-and-close, exit).

## Future portability

`image/`, `annotate.rs`, `flatten.rs`, `config.rs` are OS-free. OS-facing code is confined to
`win/*`, `capture.rs`, `overlay.rs`, `editor/*` — a future macOS/Linux port would rewrite those
layers only. This is an architectural affordance, **not** a claim of tested cross-platform support.

## Project plan authority

The **original phased execution plan** (Phases 0–6) from the project's first requirement message
is authoritative for scope and acceptance criteria. Phase numbering in `README.md` and this
document follows the executed-phase record in `docs/BUILD_LOG.md`; the mapping between the two
schemes, plus the final status of every original acceptance criterion, lives in
`docs/BUILD_LOG.md` under "Plan reconciliation". No phase is considered complete until its
original-plan exit criteria are met.
