# NameShot — Cross-Platform Plan (design only)

**Status: M0 (seam refactor) implemented and validated on Windows 2026-10-09; M1 (Linux
X11 adapters) implemented and gated 2026-10-10 — compile/test/clippy/fmt gates only, no
runtime validation (this host has no X11 session); M1b (X11 event loop/driver) implemented
and gated 2026-10-10 — same gates only, no runtime execution possible (no X server, no
WSL, no ELF run path); M1b-v (reproducible validation path + host investigation) prepared
2026-10-10 — harness written and syntax-checked but **not executed** (needs a real
Linux/X11 host), dlltool limitation root-caused as host-specific, formatting-wave audit
found no non-formatting changes; M1c/M2-M4 not started; no support claim.** NameShot today
is a Windows 10/11 x64 application; Windows is the
only platform that has been built, tested and packaged. This plan describes *how* a port
would be structured if/when it is approved — shared Rust core, small native adapters,
per-platform honesty about hard problems (Wayland in particular). The Windows package,
build scripts and artifacts are untouched by this document.

## 1. What exists today (relevant facts)

From `docs/ARCHITECTURE.md` and the current tree:

| Layer | Modules | OS coupling |
|---|---|---|
| OS-free core | `image/`, `annotate.rs`, `flatten.rs`, `config.rs`, `hotkey.rs`, `output.rs`, `session.rs`, `strings.rs`, `error.rs`, `theme.rs`, `overlay/layout.rs` | none (pure Rust, unit-tested; 114 tests green) |
| Windows layer | `win/*` (tray, hotkey, gdi, dialog, dark, clipboard, a11y), `capture.rs`, `encode.rs`, `text.rs`, `os.rs` | Win32 throughout |
| UI (Win32) | `app.rs`, `overlay/mod.rs`, `editor/*`, `settings_ui.rs` | window proc, GDI drawing, Win32 controls |

- Single dependency: `windows 0.62` (feature-gated). No `serde`, no `image`, no UI toolkit.
- M0 added the first `cfg` gates: Windows-only modules are `#[cfg(windows)]`, the
  non-Windows build binds `overlay` to its OS-free planner, and `platform/`
  (windows/linux/macos dispatch) owns the core-facing seam. Before M0 the crate was
  de-facto Windows-only by import, not by gate.
- Idle model: one hidden window + blocking `GetMessage`, ~0% CPU, no threads beyond STA COM.
- Privacy model (local-only, no network) applies unchanged to every future platform, including
  packaging decisions (no auto-updater that phones home).

The seam is real but not yet enforced: `lib.rs` documents the split, `ARCHITECTURE.md`
§"Future portability" calls it an affordance rather than a claim.

## 2. Target platforms and non-goals

Targets if approved, in suggested order:

1. **Linux / X11** — most tractable first Linux target (explicit grabs, explicit windows).
2. **Linux / Wayland** — designed and evaluated *separately* from X11; several features have
   no universal protocol and need documented fallbacks (§5).
3. **macOS** — mature platform APIs, but permission prompts (Screen Recording, Accessibility)
   and notarization dominate the work (§6).

Non-goals: BSDs, mobile, Windows-on-ARM (untested, no claim), feature parity at launch on
non-Windows platforms (core capture → annotate → export parity first; tray/hotkey/a11y may
land reduced), and any "supported" wording in README until that platform has passed a QA
pass comparable to the Windows one (visual checks, icon/tray, graceful exit, event/error
review).

## 3. Target architecture: shared core + small adapters

```
                 ┌────────────────────────────────────────────┐
                 │            platform-neutral core           │
                 │ image/ annotate flatten config hotkey      │
                 │ output session strings error theme layout  │
                 └───────────────────┬────────────────────────┘
                                     │  crate-private `platform` module
                                     │  (trait surface below, one impl per OS)
        ┌────────────────────────────┼─────────────────────────────┐
        │ win/ (existing, unchanged  │ linux-x11/                  │
        │ behaviour)                 │ linux-wayland/              │
        │                            │ macos/                      │
        └────────────────────────────┴─────────────────────────────┘
```

### 3.1 Adapter surface (deliberately small, sync, no async runtime)

Derived from actual Win32 call sites, not from aspiration:

| Adapter | Replaces | Windows impl (today) |
|---|---|---|
| `capture::grab_virtual_screen()` | `capture.rs` GDI `BitBlt` | GDI into top-down DIB |
| `encode::{png,jpeg,bmp}` | `encode.rs` WIC | `IWICBitmapEncoder` |
| `text::raster_into_frame` | `text.rs` `DrawTextW` | GDI ClearType into DIB |
| `clipboard::{copy_image,copy_text}` | `win/clipboard.rs` | `CF_DIB` + `PNG` format |
| `tray::{add,remove,modify,menu}` | `win/tray.rs` | `Shell_NotifyIconW` + `TrackPopupMenu` |
| `hotkey::{register,unregister}` | `win/hotkey.rs` | `RegisterHotKey` (1409 → non-fatal) |
| `dialog::{save_as,folder_browse,msgbox}` | `win/dialog.rs` | `IFileSaveDialog` |
| `windowing` (create overlay/editor/settings windows, paint, input, cursors) | `app.rs`, `overlay/mod.rs`, `editor/*`, `settings_ui.rs` | `CreateWindowExW` + wndproc + GDI |
| `display::{monitors,dpi,virtual_bounds}` | `os.rs` | `EnumDisplayMonitors`, PMv2 DPI |
| `paths::config_dir` | `config.rs` call into `SHGetKnownFolderPath` | `%APPDATA%\NameShot` |
| `a11y::set_name` (best-effort) | `win/a11y.rs` | `IAccPropServices` |
| `error::describe_os_code` | `error.rs` `FormatMessageW` | Win32 message lookup |

Rules:

- The trait surface stays **synchronous and boring** — no async, no callbacks with platform
  event loops leaking upward. The core keeps owning session state (`Idle → Selecting →
  Editing → Idle`).
- Mechanism: `#[cfg]`-gated module `src/platform/` re-exporting the active impl (matches the
  existing zero-dep, no-plugin style). Not a dynamic plugin system, not `dyn` everywhere:
  each adapter is a plain module with free functions/structs the UI layer already resembles.
- GDI RAII discipline in `win/gdi.rs` is the pattern to copy per platform (handles released
  on success, cancel and error paths).
- UI windows stay **native per OS** (Win32 / X11-Xlib / layer-shell wl_surface /
  NSWindow). See §7 for why no cross-platform UI framework is proposed.

## 4. Linux / X11 plan

| Concern | Approach | Notes |
|---|---|---|
| Capture | `XGetImage` on the root window over the union of monitor geometries, or XShm + composite; convert Z/24bpp → BGRA | One-shot per session, matching current model. RandR (`XRRGetMonitors`) for monitor list. X11 has no DPI-virtual-screen negatives — coordinates are root-relative |
| Overlay | override-redirect topmost window spanning the union rect, `_NET_WM_WINDOW_TYPE_DOCK`-free; grab pointer during drag (`XGrabPointer`) | Mirrors `WS_EX_TOOLWINDOW` "don't show in taskbar/alt-tab" |
| Editor/Settings | plain top-level windows + custom drawing (same owner-draw approach as today) | Toolkit-free keeps the dependency graph honest; must verify focus/hover parity manually |
| Tray | StatusNotifierItem over D-Bus (`zbus`), with xembed fallback for bare WMs | SNI is the modern path; xembed covers openbox/fluxbox users |
| Hotkey | `XGrabKey` on the hidden app window | Conflicts with WM bindings — treat "already grabbed" like Win32 error 1409 (non-fatal, report in Settings) |
| Clipboard | `CLIPBOARD` selection ownership; PNG via `image/png` MIME target + `image/bmp` DIB-equivalent if needed | Owning a selection means implementing the transfer protocol (event loop iteration) |
| Save/folder dialogs | `rfd` with the GTK backend, or portal backend (§5) | rfd is the one intentionally-added dep candidate; evaluate size vs. hand-rolled zenity call |
| Text raster | shared raster font stack (§6) | ClearType has no X11 equivalent; subpixel order is per-monitor — default to greyscale AA, document the visual delta |
| a11y | AT-SPI via `atspi` (optional feature) | best-effort, same "no claim" posture as Windows |
| DPI | Xft.dpi / RandR scale read once at startup; coordinates stay physical pixels where available | X11 fractional scaling is compositor-dependent; record as open question |

## 5. Linux / Wayland plan (designed separately — this is the hard one)

Wayland by design gives clients no global power: no pointer warping, no global key grabs,
no arbitrary screen reads, no always-on-top layer without a protocol. The plan is explicit
about what works everywhere, what needs protocols, and what falls back.

| Concern | Primary | Fallback / honest limitation |
|---|---|---|
| Capture | **xdg-desktop-portal** `Screenshot` (one-shot, user consent, compositing handled by the desktop) — fits the one-shot-per-session model exactly | Interactive region: portal `Interactive=1` lets the *compositor* draw the selection (loses our overlay UX). Screencast portal only if per-frame capture is ever needed (it is not, today) |
| Selection overlay | **wlr-layer-shell** (`zwlr_layer_shell_v1`) top-level, keyboard-interaction-exclusive, fullscreen-ish overlay on wlroots/KDE/Sway | **GNOME/Mutter has no layer-shell.** Options, in order of preference: (a) portal interactive selection on GNOME, (b) a normal `xdg_toplevel` fullscreen window (appears in overview/alt-tab, stacking not guaranteed), (c) ship the full overlay experience on KDE/Sway/hyprland and document the GNOME limitation. Must be validated empirically before any claim |
| Tray | SNI via `zbus` (`org.kde.StatusNotifierWatcher`) | GNOME needs the AppIndicator extension; document, don't fight the DE |
| Hotkey | `global-shortcuts` portal (KDE/GNOME implementations differ and may be absent) | Fallback: expose "trigger via tray/CLI/`nameshot --capture`" and let the DE assign a shortcut; never pretend `XGrabKey` exists |
| Save/folder dialogs | portal `FileChooser` via `ashpd`, or `rfd`'s portal backend | Portal path also satisfies Flatpak sandboxing |
| Config dir | `XDG_CONFIG_HOME`/`dirs`-style resolution (hand-rolled to stay dep-light, or tiny `dirs`-equivalent logic) | keep the INI format byte-compatible across OSes |
| a11y | AT-SPI (same as X11) | best-effort |

Design consequences to accept up front:

- The "quiet idle, zero threads" Windows model survives (portals are D-Bus calls on demand),
  but the message loop becomes toolkit-specific: X11 = `XPending`/`XNextEvent`, Wayland =
  `wl_display_dispatch` — both behind the `windowing` adapter, no async runtime.
- Capture UX differs on GNOME (compositor-drawn selection) vs KDE/Sway (our overlay). The
  Settings dialog must not promise identical UX per DE until tested.
- Fractional scaling + mixed-DPI on Wayland: buffers are scaled by the compositor;
  logical↔physical mapping must go through scale factors — same "physical pixels" goal as
  Windows PMv2, different mechanism.

## 6. macOS plan

| Concern | Approach |
|---|---|
| Capture | **ScreenCaptureKit** (`SCScreenshotManager`, macOS 12.3+; fall back to `CGWindowListCreateImage` on older, noting its deprecation) — one-shot, matches session model |
| Permissions | **Screen Recording** TCC prompt appears on first capture; app must handle denial (guide user to System Settings → Privacy). **Accessibility** TCC for global hotkeys if using `AXUIElement`/event taps; `RegisterEventHotKey` itself does not require it — verify empirically. Hardened runtime + notarization do not change TCC flow but require stable bundle id for grants |
| Overlay | Borderless `NSWindow` at `.screenSaver`/`mainMenu+1` window level over the captured image; can't cover the menu bar or secure login content (same class of limitation as Windows secure desktop) |
| Tray | `NSStatusItem` in the menu bar (macOS convention replaces "tray icon") |
| Hotkey | `RegisterEventHotKey` (system-wide, conflict reporting via `GetEventParameter`) |
| Clipboard | `NSPasteboard` PNG + TIFF types |
| Save/folder dialogs | `NSSavePanel` (via `rfd` cocoa backend or direct objc calls) |
| Text raster | CoreText rasterization of the same font stack (§6.1) or shared raster crate — pick one implementation and use it on all OSes if quality parity allows |
| a11y | `NSAccessibility` attributes on controls (same best-effort posture) |
| Packaging | Universal (x86_64 + aarch64) or separate builds; DMG; notarization + stapling; login item via `SMAppService`; **no auto-updater** (local-first) |

### 6.1 Text and encode — the hidden Win32 inside "almost core"

Two modules are portable in spirit but Win32 in fact:

- `text.rs` (GDI `DrawTextW`) — needs a shared text stack. Candidates: `ab_glyph` (small,
  no system-font integration) or `fontdue` (fast rasterizer); system font *loading* per OS
  (DirectWrite / CoreText / fontconfig) stays in the adapter. Tradeoff: we lose ClearType
  subpixel filtering; greyscale AA is the portable baseline. Measure quality on the existing
  text-annotation tests before choosing.
- `encode.rs` (WIC) — candidates: `png` crate (pure Rust, ubiquitous, decode used already by
  our byte-exact tests), `zune-jpeg` or `jpeg-encoder` for JPEG, BMP is trivial to keep
  hand-written. Tradeoff: WIC's JPEG is Win32-native; encoder output will differ byte-wise —
  existing byte-exact header tests must become per-backend golden tests, output *semantics*
  (dimensions, color type, round-trip equality) stay asserted on all platforms.

## 7. Dependencies and heavyweight-toolkit tradeoffs

Proposal discipline: **every new dependency must be justified in this table before adoption,
with measured binary-size and idle-CPU impact on Windows kept as the regression bar.**

| Candidate | Used for | Verdict |
|---|---|---|
| `png` (+ maybe `zune-jpeg`, `jpeg-encoder`) | portable encode | **Proposed** — pure Rust, no unsafe, replaces WIC off-Windows only (Windows keeps WIC until a measurement says otherwise) |
| `ab_glyph` or `fontdue` | portable text raster | **Proposed, gated on a quality spike** — one shared implementation beats three native text stacks if quality passes |
| `zbus` / `ashpd` | Wayland tray/portals | **Proposed for Wayland only**, feature-gated; X11 build must not link it |
| `rfd` | save/folder dialogs on Linux/macOS | **Proposed, evaluated** — vs. hand-rolled per-OS; it is the largest dep candidate, measure first |
| `winit` + `wgpu`/`softbuffer` | cross-platform window+GL | **Not proposed now.** It would unify event loops but replaces the entire UI layer with a non-native one; the current architecture is "native windows + custom draw" which winit does not provide (controls, dialogs, menus). Revisit only if Wayland overlay work shows native integration failing — with numbers |
| `tauri`/`egui`/`iced`/GTK/Qt app shells | whole-app UI | **Rejected for now** — browser-runtime or heavyweight-toolkit cost contradicts the project's stated goals (no web view, ~0 idle CPU, minimal deps); would also invalidate the existing Win32 UI investment. Explicitly a measured-tradeoff decision: adopt only if porting cost of native UI is proven worse than framework cost |
| async runtime (`tokio` etc.) | portal/D-Bus loops | **Not proposed** — sync adapters + the existing single event loop keep the idle model; portals are request/response |

Windows packaging, ZIP layout and the `windows` crate feature list stay exactly as they are.

## 8. Testing and verification strategy

1. **Keep the OS-free suite green everywhere** — the 107 tests (raster, redaction round-trip,
   layout `plan()`, INI, hotkey parse, filenames) run on any host unchanged; that is the
   core's portability proof.
2. **Adapter smoke tests per OS**, behind `#[ignore]` or features (capture→encode→decode
   round-trip, clipboard round-trip where a clipboard exists, hotkey register/unregister)
   — runnable in CI with Xvfb on Linux, headless-limited on macOS.
3. **CI matrix (build + unit only)**: `windows-latest`, `ubuntu-latest` (Xvfb), `macos-latest`.
   Clippy/fmt gates copied from the Windows workflow.
4. **Manual QA per platform before any wording change**: a `qa_*` folder equivalent of the
   Windows visual pass (selection scenarios A–D style, tray icon, About/Exit, graceful-exit
   event review). Windows QA scripts remain Windows-only; per-OS scripts are written when
   that OS's port starts, not speculatively.
5. **No support claim** until 4 passes on that platform; README/docs wording changes are a
   separate, explicit approval step.

## 9. Packaging

| Platform | Artifact | Notes |
|---|---|---|
| Windows | unchanged (`dist\NameShot-0.1.0-win-x64-*.zip`, icon/version resources via `build.rs` windres) | out of scope for this plan beyond regression checks |
| Linux | `.deb` + `.rpm` + AppImage (glibc x86_64 first); Flatpak later | AppImage covers X11+Wayland with one binary; Flatpak must declare `org.freedesktop.portal.Screenshot`/`FileChooser` permissions and talk to portals only |
| macOS | notarized DMG (universal), hardened runtime, bundle id stable for TCC | no Sparkle/network updater — updates stay manual, consistent with local-only claims |

Desktop entry, icon theme install (`hicolor`), and `nameshot --capture` CLI entry are Linux
additions owned by the packaging phase, not the core.

## 10. Phasing (each phase gated; nothing starts without approval)

| Phase | Content | Exit criteria |
|---|---|---|
| **M0 — seam refactor (Windows only)** | Introduce `#[cfg]`-gated `platform/` module, move existing Win32 calls behind it *without behaviour change*; add `cfg` gates so `cargo check --target x86_64-unknown-linux-gnu` fails only in `platform/windows` (expected), core compiles | 107 tests green, clippy/fmt unchanged, exe byte-comparable or diff-explained, Windows QA smoke re-run |
| **M1 — Linux X11 MVP** | capture, overlay select, editor annotate/flatten, encode, save+clipboard, tray(SNI), hotkey(XGrabKey), settings | core tests + new smoke tests green; manual pass; docs updated without support claims |
| **M2 — Wayland** | portals for capture/dialogs, layer-shell overlay where available, DE-documented fallbacks (§5) | same, plus written DE-compatibility matrix (KDE/Sway/GNOME tested) |
| **M3 — macOS** | ScreenCaptureKit + TCC flows, NSWindow overlay, NSStatusItem, event hotkeys, notarized DMG | permission-denial UX verified; manual pass |
| **M4 — packaging polish** | Flatpak, desktop files, universal macOS build | release artifacts reviewed |

Phases M1–M4 are contingent on M0 being validated as zero-behaviour-change on Windows.

**M0 result (2026-10-09) — implemented, validated, no behaviour change on Windows:**

- **Seam:** new `src/platform/{mod,windows,linux,macos}.rs` (cfg-dispatch; Windows
  re-exports `crate::text::draw_text`, Linux/macOS stub it `unimplemented!` for M1);
  `#[cfg(windows)]` on `app`, `capture`, `editor`, `encode`, `os`, `overlay`, `settings_ui`,
  `text`, `win`; non-Windows `overlay` binds to `overlay/layout.rs`; `Error::Win32` variant
  and its `From`/match arms cfg-gated; core `layout::Rect` replaces `RECT` with `From`
  conversions at the `overlay/edit` boundary; `windows` dep target-gated in `Cargo.toml`
  (`build.rs` was already target-gated).
- **Gates:** `cargo check --target x86_64-unknown-linux-gnu` and
  `--target aarch64-apple-darwin` exit 0, zero warnings; `cargo test` 114/114; clippy exit 0
  (33 lib warnings = baseline, none in changed files); `cargo fmt --check` exactly 228 = 228
  baseline (`overlay/layout.rs` still 0).
- **Exe diff-explained:** release exe no longer byte-identical (`676AE653...` vs
  `6A9B25E5...`, +21 bytes); all 11 PE sections have byte-identical sizes, 4654/4654
  functions present, 4644/4645 same size; the 9 renamed symbols are the `overlay::edit`
  methods whose impl-block disambiguator shifted from the two new `From` impls; remaining
  byte churn is relocation (RIP-relative displacements after link-order change).
- **Windows QA smoke** (`qa_20261009_185813_M0_smoke`): scenarios D and A match baseline
  pixel-for-pixel at the chrome (chip `#20242C` at identical coords; "331 x 208"; rail/strip
  above the y1160 taskbar band), tray overflow flyout + hover OK, graceful exit, explorer
  PID 5132 unchanged, no error events.
- **Host limitation:** this machine has no WSL, so Linux/macOS can only be `cargo check`ed
  (no linker, no GUI) until M1 runtime work is separately approved.

**M1 result (2026-10-10) — X11 adapters implemented and gated; runtime untested:**

- **Delivered:** the nine X11 adapter modules behind the M0 seam (`platform/x11/*` via
  `linux.rs`) — capture, encode (PNG via `png`, BMP via core `build_bmp`), ICCCM
  clipboard, `XGrabKey` hotkeys with conflict mapping, RandR 1.5 display geometry,
  windowing primitives, zenity-backed file/message dialogs, `ab_glyph` text raster —
  plus pure protocol logic `platform/x11_logic.rs` (unit-tested in the regular Windows
  suite) and the XDG config split. **Not in this pass:** M1b UI/event loop, tray
  (§4 zbus vs §7 no-zbus — needs a decision), Wayland/macOS/packaging, a11y, Linux JPEG
  (clean error).
- **Deps (Linux-target only, pure Rust):** `x11rb 0.14` (feature `randr`), `png 0.18`,
  `ab_glyph 0.2` — no libX11, bindgen, system libs, `zbus`, `rfd`, or async runtime;
  Windows resolution unchanged.
- **Gates (all in BUILD_LOG M1 with exact commands):** linux + macos `cargo check` exit 0
  (incl. `--tests` on linux); `cargo test` **132/132** (M0 baseline 114 → +18); windows
  clippy exit 0 at exact baseline 33/39; linux clippy exit 0 with **zero warnings in
  `platform/x11*`** (4 M1 warnings fixed); `cargo fmt --check` exit 1 at exactly
  **228 = 228 baseline** (new files rustfmt-clean); `cargo build --release` exit 0;
  musl attempted — deps + lib codegen OK, final ELF link fails (`cc` missing on host).
- **Exe:** raw SHA never reproducible (twin same-source builds differ only in the PE
  `TimeDateStamp` + `CheckSum`, 2 bytes); content diff-explained against the packaged
  `6A9B25E5` exe as **exactly M0's documented `676AE653` delta** (+21 bytes, identical
  11 section raw sizes incl. icon `.rsrc`, 0 size diffs across 5515 common symbols,
  `rebuild_bar` 3664→3648, 11 rename/disambiguator pairs); M1-only additions are cfg'd
  out of the Windows exe (`cfg(any(test, not(windows)))`).
- **Runtime:** none — no X server/WSL here; every adapter's runtime behaviour and all
  Linux resource numbers remain unmeasured (BUILD_LOG M1 “Not tested”).

### M1b result (event-loop/runtime phase, 2026-10-10 — compile-verified only)

- **Implemented:** `SelectSession` + event-mapping policy (`x11_logic.rs`, x11rb-free,
  16 new unit tests: Windows-mirrored drag/click/Enter/Esc semantics, veil/compose math,
  root→frame mapping); `x11/eventmap.rs` (x11rb field extraction, cfg linux);
  `x11/app.rs` (connect → veil → focus → grab → loop → crop → zenity → PNG);
  `keycode_for_keysym` extraction; `set_input_focus`; `lib.rs run()` split
  (windows/linux/other). Out of scope: move/resize handles, editing, hotkey registration,
  clipboard-after-save, tray.
- **Gates (BUILD_LOG M1b, exact commands):** `cargo test` **148/148** (132 → 148);
  linux/macos `cargo check` + linux `--tests` exit 0; windows clippy **33/39 = exact
  baseline**; linux clippy **11/17 = exact baseline**, 0 in `platform/x11*`; fmt exit 1 at
  exactly **1 pre-existing hunk** (`tests/encode_roundtrip.rs:11`; an external formatting
  wave at 02:01 reduced the 228-baseline); `cargo build --release` exit 0.
- **Windows exe:** rebuilt copy proven code-identical to the pre-M1b build — `.text`,
  `.data`, `.pdata`, `.xdata`, `.idata`, `.reloc`, `.rsrc` bytes identical; only line/col
  slots in `.rdata` and symbol-hash suffixes differ (span churn from the formatting wave);
  `dist\nameshot.exe` (`6A9B25E5`), 4 ZIPs, PID 9420 untouched. (An x11rb *dev-dependency*
  attempt failed on this host — rustc's `dlltool --temp-prefix kernel32.dll:` spawn is
  broken here — and was reverted; mapping policy was moved into the tested core instead.)
- **Runtime:** none — determined before coding: no WSL, no X server/tools, `DISPLAY` empty,
  no ELF linker/execution path. The driver has **never run**; no runtime numbers; no
  Linux support claim. Full record + stop point: BUILD_LOG “M1b”.

### M1b-v result (validation path + host investigation, 2026-10-10 — prepared, not executed)

- **Reproducible path:** `scripts/linux_x11_validate.sh` for a real Linux/X11 host —
  T0 env capture, T1 compile tier, T2 unit tier (expected **156** = 146 lib incl. 8 new
  Linux-gated `eventmap` tests + 10), T3 live tier kept strictly separate (capture,
  selection, window lifecycle auto-recorded + screenshots; hotkeys/global and
  clipboard-after-save honestly marked NOT IMPLEMENTED in M1b). Verified here only as
  `bash -n` exit 0 + non-Linux guard exit 2; **never executed** — no runtime claim.
- **dlltool/x11rb limitation: host-specific** (verbatim probes in BUILD_LOG M1b-v): the
  self-contained binutils `dlltool` spawns a sibling `as` that rust-mingw does not ship
  (manifest-proven), and whatever provided a working pair on 2026-10-07 (cached
  `*.dll_imports.lib`, 10-07 17:48) is gone; P0 empty crate passes, P1-P3 raw-dylib fail
  with captured errors. NameShot's Windows build unaffected; **Linux path unaffected**
  (no dlltool on Linux). No dependency was removed/replaced for the host; no toolchain
  changes made (repair proposal only, needs approval).
- **Formatting wave:** audited via panic-`Location` decode + byte-identical code sections —
  12 files, all wave members, formatting-only signatures; **no non-formatting changes to
  compiled code**; residual limit: comment-level edits in wave files are unprovable without
  VCS (recommend establishing a git baseline).
- **Compile evidence added:** `cargo check --target x86_64-unknown-linux-gnu --tests` exit 0
  (8 new extraction tests compile); windows suite 148/148, both clippys at exact baselines
  (33/39, 11/17), fmt exit 1 = the one known hunk.

## 11. Open questions / risks (must be answered by spikes, not assumptions)

1. **GNOME overlay**: no layer-shell — is portal `Interactive` selection acceptable UX, or is
   a plain fullscreen toplevel acceptable? (spike before M2 commitment)
2. **Wayland global hotkeys**: how many target DEs actually implement `global-shortcuts`?
   Fallback UX (DE-assigned shortcut + tray) must be designed, not discovered late.
3. **Text quality delta**: greyscale AA vs ClearType — acceptable for text annotations on
   Windows-parity screenshots? (spike in M0/M1)
4. **Encoder parity**: `png` crate vs WIC byte output; JPEG quality settings mapping.
5. **Mixed-DPI on Wayland/X11** vs Windows PMv2 physical-pixel guarantees.
6. **Tray presence on GNOME** (extension dependency) — messaging without sounding broken.
7. **Binary size / idle CPU regression bar**: record current Windows numbers before adding
   any dependency, then enforce them.
8. `rfd` vs. hand-rolled dialogs — decide with measured maintenance cost.

## 12. Summary

The OS-free core plus `win/` boundary already anticipates this plan; the work is (a)
enforcing that seam behind a `cfg`-gated `platform` module with zero behaviour change,
(b) adding three small native adapter sets, (c) accepting — and documenting — that Wayland
cannot promise full UX parity without compositor cooperation, and (d) keeping the
dependency list as short as Windows' current one unless a spike proves otherwise.
**No porting, packaging or production-code work has started; this document awaits review.**
