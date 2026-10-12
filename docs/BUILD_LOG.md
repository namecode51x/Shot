# NameShot — Build Log

Chronological record of phases. Every entry states what was **actually run and observed**.

---

## Phase 0 — Repository and toolchain reconnaissance (2026-10-06)

### Repository

- `D:\Proyectos\NameShot` existed and was **empty** (0 files, not a git repository).
  Nothing to preserve; no unrelated files touched.

### Machine / OS

| Item | Observed value | Source |
|---|---|---|
| OS | Windows 10 IoT Enterprise LTSC, `Microsoft Windows NT 10.0.19044.0` (build 19041/19044) | `Get-CimInstance Win32_OperatingSystem`, `[Environment]::OSVersion` |
| Arch / CPU | x64, Intel Core i5-3470 @ 3.20 GHz, 4 cores | `Win32_Processor` |
| RAM | 16 270 MB | `Win32_ComputerSystem` |
| GPU | Intel HD Graphics | `Win32_VideoController` |
| GUI available? | **Yes** — session 1, `explorer.exe` running (PID observed) | `SessionId`, `Get-Process explorer` |
| PowerShell | 5.1 (Windows PowerShell) | `$PSVersionTable` |
| Language | Spanish Windows (messages localized; logs keep English) | observed |

Conclusion: this machine **can run and be measured with a real Windows GUI**, so Windows-specific
behaviour (tray, hotkey, overlay, editor, clipboard, dialogs) can be exercised directly.

### Toolchain state at start

- `rustc`, `cargo`, `rustup`: **not installed** (not on PATH, `~/.cargo/bin` absent).
- Visual Studio / MSVC (`vswhere.exe`, `link.exe` from MSVC): **not installed**.
- `winget`, `choco`, `scoop`: not available. `gcc`/`clang`: not present anywhere on disk.
- `git`: available (`C:\Program Files\Git\cmd\git.exe`).
- Network: reachable. Measured throughput is low (≈50–110 KB/s to most hosts), so the toolchain
  was chosen to minimise download size.

### Toolchain installed (during Phase 0)

1. `rustup-init.exe -y --default-toolchain stable --default-host x86_64-pc-windows-gnu --profile minimal`
   (first run was interrupted → toolchain reported `missing manifest`; fixed with
   `rustup toolchain install stable --profile minimal`).
   - **Verified:** `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0 (5f94df478 2026-08-27)`,
     host `x86_64-pc-windows-gnu`; components present: `rustc`, `cargo`, `rust-std`, `rust-mingw`,
     `rustfmt`, `clippy`, `rust-docs`.
2. MinGW-w64 (needed as the linker for the GNU host): winlibs
   `x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0` zip (274 MB) downloaded from the official
   GitHub release to `%TEMP%\opencode\nameshot-setup\mingw.zip` (resumed with `curl -C -`).
   - Rationale: MSVC route requires Visual Studio Build Tools + Windows SDK (~5 GB) and none are
     installed; MSYS2 mirrors measured ≈50 KB/s; GitHub release measured ≈110 KB/s.
   - Verification of the installed gcc is recorded in Phase 1.

### Decisions taken in Phase 0

- **Language/toolchain:** stable Rust, `x86_64-pc-windows-gnu` host/target, `windows` crate for
  Win32. (Material environment constraint — no MSVC on the machine; documented here because it
  affects build commands in the README.)
- **Architecture** written to `docs/ARCHITECTURE.md` *before* substantial implementation:
  module boundaries, Win32 API choices (GDI one-shot capture vs DXGI evaluation, WIC encoding,
  CF_DIB clipboard, IFileSaveDialog, INI settings), redaction-correctness model, dependency policy
  (only the `windows` crate).
- **No unsupported platform assumptions:** the app is Windows-first; cross-platform code is only
  the OS-free core, and no untested platform claim will be made.

### Exit criteria (Phase 0)

- [x] Repository structure and build environment understood.
- [x] `docs/ARCHITECTURE.md` written before implementation.
- [x] OS is Windows 10 x64 with GUI → Windows behaviour will be actually run, not simulated.

---

## Phase 1 — OS-free core (2026-10-06 → 2026-10-07)

### Implemented

Pure-Rust, no-Win32 modules (unit-testable without a GUI session):

| Module | Responsibility |
|---|---|
| `strings.rs` | All user-facing text (EN), `fill()` placeholder substitution |
| `error.rs` | `AppError` type + OS-error formatting |
| `image/mod.rs` | `Frame` (BGRA), `IRect`, `VirtualScreen`, geometry/colour helpers |
| `image/redact.rs` | Redaction/blur primitives operating only on raw pixels |
| `annotate.rs` | Shapes, pen, highlight, text layout model (renderer-agnostic) |
| `flatten.rs` | Annotation → pixel rasterisation over a `Frame` |
| `config.rs` | `Settings` model + INI load/save (`%APPDATA%\NameShot\settings.ini`) |
| `hotkey.rs` | `HotkeySpec`, modifiers, key names, Win32 registration **format** logic |
| `output.rs` | Save-dir resolution, unique filenames, format selection |

### Actually run and observed

```
$ cargo test
test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

- Exit code 0; 46/46 tests pass (2026-10-07, re-run after UI code was added: still 46/46).
- `cargo check --all-targets` also compiles the `#[cfg(test)]` targets: clean.

### Exit criteria (Phase 1)

- [x] Core logic implemented with no `windows`-crate dependency in the modules above.
- [x] `cargo test` green (46 passed / 0 failed), reproduced on a later run.

---

## Phase 2 — Windows layer and full application UI (2026-10-06 → 2026-10-07)

### Implemented

| Module | Responsibility |
|---|---|
| `win/gdi.rs` | `this_module()`, pens/brushes/fonts, `draw_label`, stock objects |
| `win/dark.rs` | DWM dark titlebar, shared `COLORREF`s, window-background brushes |
| `win/hotkey.rs` | `RegisterHotKey`/`UnregisterHotKey` + conflict detection |
| `win/clipboard.rs` | CF_DIB copy / paste |
| `win/tray.rs` | `Shell_NotifyIconW` add/modify/delete + context menu |
| `win/dialog.rs` | `IFileSaveDialog`, folder picker, message boxes |
| `win/mod.rs` | module glue, `wide()` UTF-16 helper |
| `capture.rs` | GDI `BitBlt` one-shot capture of the virtual screen |
| `encode.rs` | WIC PNG/JPEG/WEBP encode, `get_dibits`-based decode |
| `text.rs` | DirectWrite-ish GDI text measurement for the editor |
| `os.rs` | Windows 10/11 detection, paths |
| `app.rs` | `App` state, tray hotkey dispatch, capture/save/exit flows, single `GetMessage` loop, custom message prefilter, window-class registration, fonts |
| `overlay.rs` | Full-screen capture overlay: rubber-band selection, handles, pan/resize, keyboard |
| `editor/mod.rs` | Dark editor: tools, colour/width strip, history (undo/redo), zoom/pan, text-edit child, status bar, save/copy |
| `editor/toolbar.rs` | Custom-painted vertical tool strip + colour/width controls |
| `settings_ui.rs` | Modeless Settings window, hotkey-capture window, About window |

### Actually run and observed (2026-10-07)

```
$ cargo check --all-targets   # exit 0, 0 errors, 0 warnings
$ cargo test
test result: ok. 46 passed; 0 failed; ...
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.08s
→ target\debug\nameshot.exe, 20 445 159 bytes
```

Full warning sweep done in this phase: **0 errors / 0 warnings** after fixing every
`unused BOOL` (`InvalidateRect`/`EnableWindow`/`Ellipse` results now bound with `let _ =`),
private-type-in-public-interface (`Drag`/`ODrag` → `pub`), unused imports/consts/params.

### Technical findings recorded during Phase 2

- `windows` 0.62: `SendMessageW`/`PostMessageW` take `Option<WPARAM>`/`Option<LPARAM>`;
  `SetForegroundWindow`, `EnableWindow`, `InvalidateRect`, `Ellipse` return must-use `BOOL`.
- `RefCell<Option<T>>::borrow().map(...)` fails with E0507 (moves out of `Ref`) →
  must be `.borrow().as_ref().map(...)`; the compiler suggestion is reliable.
- Edition 2024: unsafe ops inside `unsafe fn` (our Win32 wndprocs) are **warnings** by default
  (`unsafe_op_in_unsafe_fn`) but hard **errors** (E0133) inside *safe* fns → each such call site
  in safe code was wrapped in `unsafe { }`. Crate-level `#![allow(unsafe_op_in_unsafe_fn)]`
  documents the wndproc convention.
- Custom window classes must pass `Some(this_module())` as `hinstance`, otherwise
  `LoadIconW`/cursor lookups silently fall back to the exe module of the *other* process.
- Message prefilter design finalised: keyboard messages only; hotkey-capture window redirects
  `msg.hwnd` to its root (it reads raw keys itself); Settings/About delegate to
  `IsDialogMessageW(root, msg)` which uses `WM_GETDLGCODE` and posts `IDOK`/`IDCANCEL`
  (mapped to `CMD_OK`/`CMD_CANCEL` = 1/2).

### Exit criteria (Phase 2)

- [x] All planned modules implemented; `cargo check --all-targets` clean (0/0).
- [x] `cargo build` links `nameshot.exe`.
- [ ] GUI behaviour (tray, hotkey, overlay, editor, settings, clipboard, save) — **not yet
      exercised**; deferred to Phase 3, no claim of working UI is made here.

---

## Phase 3 — Live GUI verification on the real machine (2026-10-07)

### Test approach

Interactive but hands-off: windows were driven with `PostMessageW`/`SendMessageW` (packed
`lParam=(y<<16)|x` for coordinates) and `keybd_event` only while one of *our* windows/menus
had focus; **no cursor movement, no mouse clicks**. Screenshots taken passively with GDI
`CopyFromScreen` from a separate PowerShell process (`SetProcessDPIAware()` first) — verified
this session to return live pixels (fresh capture matched wall clock exactly).

Scripts (in `%TEMP%\opencode`): `ns_save.ps1` (dedicated save-path probe) and `ns_phase3.ps1`
(canonical A–K sequence; `ns_diag.ps1` was an earlier instrumented attempt, superseded).

Message IDs used: `WM_HOTKEY=0x0312` (wParam `0x4E53`) to `NameShot.Main`, tray callback
`WM_APP_TRAY=0x8001` lParam `0x205` (activate → menu), `WM_CLOSE=0x0010`, `WM_KEYDOWN=0x0100`,
mouse `WM_MOUSEMOVE/LBUTTONDOWN/LBUTTONUP` for the rubber band.

### A–K sequence — final run (13:07:20 → 13:07:31, process exit code 0)

| Step | Action | Observed |
|---|---|---|
| A | Launch `nameshot.exe` | Main window class `NameShot.Main` found, PID recorded, **no console window** |
| B | `PostMessage(WM_HOTKEY)` | Capture overlay window appears, full-screen |
| C | `WM_LBUTTONDOWN (600,300) → WM_MOUSEMOVE (1500,850) → WM_LBUTTONUP` | Teal rubber band + 8 handles + hint strip visible in `p3_c_region.png` |
| D | `WM_KEYUP VK_RETURN` | Editor opens with the 901×551 region; overlay destroyed |
| E | `Ctrl+C` (real `keybd_event`, editor foreground) | Clipboard contains `CF_DIB`, size **1 985 844 B** |
| F | `Ctrl+S` | **"Guardar como" dialog (`#32770`, pid-filtered) found; default filename `NameShot_2026-10-07_130723` selected; Enter → saved**: file count 0→1 |
| G | After save | Editor not dirty (save clears it); `p3_f_savedialog.png` shows the dialog + `Copied to clipboard` status + clock 13:07 (matches run exactly); in-app success status `Saved: NameShot_2026-10-07_130723.png` confirmed in the `ns_save` run |
| H | Tray menu via `PostMessage(hwnd, WM_APP_TRAY, 0, 0x205)` | Menu `#32768`: Capture / sep / Settings / About / sep / Exit; `Down,Down,Enter` → **Settings window opened** |
| I | `Esc` in Settings | Closed by prefilter (`IsDialogMessageW` path), app still running |
| J | Tray → `Down,Down,Down,Enter` → **About**, `Esc` | About window opened and closed |
| K | Tray → Exit | Process exited cleanly, **no panic file** |

Saved file verified independently: **162 182 B, 901×551, valid PNG** (decoded with
`System.Drawing`), then deleted — `Pictures` folder left empty (test files are removed after
verification, per privacy rule).

### Independent save-path verification (`ns_save.ps1`, 12:42 run)

Because detection bugs had previously made step F ambiguous, a dedicated script retried the
whole Ctrl+S path with EnumWindows-based dialog polling and a post-Enter screenshot:

- Dialog `dialog=2888088` found, `dialog_still_open=False` after Enter.
- `p3s_b2_postenter.png` status bar reads **`Saved: NameShot_2026-10-07_124218.png`** —
  success path observed end-to-end in the app's own UI.
- File created (before=0 → after=1), clean exit, no panic; file deleted afterwards.

### Bugs found in this phase (all fixed, verified by re-run)

| # | Bug | Fix |
|---|---|---|
| 1 | `encode_to_file` produced **0-byte PNGs**: `CreateNewFrame` was not followed by `frame_enc.Initialize(&bag)` → `SetSize` failed with `WINCODEC_ERR_PROPERTYNOTINITIALIZED (0x88982F0C)` | `src/encode.rs`: initialize the frame encoder with the properties bag; bag now unwrapped with `ok_or_else` |
| 2 | `encode_png_bytes` cast the HGLOBAL shell stream to `IWICStream` → `E_NOINTERFACE (0x80004002)` (shell streams don't implement `IWICStream`) | pass a plain `IStream` to `encode_into` (encoders accept it); `decoder_from_bytes` now uses `fac.CreateStream()` + `InitializeFromMemory` |
| 3 | `IFileSaveDialog` worked but the process never called `CoInitialize[Ex]` — behaviour relied on an unexplained implicit init | explicit `CoInitializeEx(None, COINIT_APARTMENTTHREADED)` at the top of `app::run()` (all results incl. `RPC_E_CHANGED_MODE` ignored); test helpers call the same |

A regression test was added: **`tests/encode_roundtrip.rs`** — 4 integration tests
(PNG/JPEG/BMP `encode_to_file` → non-empty → `decode_file` round-trip; `encode_png_bytes`
signature + round-trip), each with its own `init_com()`.

```
$ cargo test        → 46 unit + 4 integration = 50 passed, 0 failed
$ cargo check --all-targets → exit 0, 0 warnings (re-confirmed 13:15)
$ cargo build       → target\debug\nameshot.exe rebuilt clean
```

### Test-harness findings (PowerShell, not app bugs)

- `Write-Output` **inside an `EnumWindows` callback delegate is silently lost** → collect into
  a `$script:` list, flush after the call.
- `FindWindowExW(Zero, Zero, '#32770', $null)` never matches titled dialogs in PS → EnumWindows
  + `GetClassNameW` + pid filter (`FindOurDlg`) instead.
- Structs declared inside `-MemberDefinition` are nested: `New-Object D2.W+RECT`.
- PowerShell functions emit all output: assigning `$x = Wait-Function` captured status strings
  alongside the handle → poll loops must write status to `$script:` or `[void]`.
- `Set-Content`/heredocs corrupted scripts earlier — edits done with the file-edit tool from
  then on.

### Hotkey note

Default hotkey `PrintScreen` (`RegisterHotKey` with no modifiers) intermittently fails with
**error 1409 `ERROR_HOTKEY_ALREADY_REGISTERED`** — explorer's snipping hook holds the key.
Registering with `MOD_ALT` succeeds. Explorer still holds plain PrintScreen on this machine,
so the *global* registration could not be observed here; the handler path was exercised by
posting `WM_HOTKEY` directly (step B), which is the same code path a registered hotkey fires.
Settings UI allows changing the hotkey; conflict detection reports the failure.

### Screenshot inventory (in `%TEMP%\opencode`, final run)

`p3_b_overlay`, `p3_c_region`, `p3_d_editor`, `p3_f_savedialog` (dialog + correct default
filename + clock 13:07), `p3_h_menu`, `p3_i_settings`, `p3_j_about` — plus `p3s_*` from the
independent save run (editor, dialog, post-Enter "Saved:" status, after-save). Earlier
`p3x_*`/`p3s_c_statusbar` files are from superseded runs kept for comparison; resolved as
harness detection issues, not application defects.

### Exit criteria (Phase 3)

- [x] Tray, hotkey path, overlay, region selection, editor, clipboard, native Save dialog,
      Settings, About, clean exit — all exercised live on Windows 10 x64 (A–K pass).
- [x] Every defect found was a real bug; fixed and covered by tests (50 green).
- [x] No unverified UI claim: screenshots for each step retained; save verified twice
      (file bytes + in-app status message).
- [ ] Global `PrintScreen` registration — blocked by explorer's own hotkey (1409);
      handler verified via posted `WM_HOTKEY`. Documented, not claimed.

---

## Phase 4 — Icon embedding, README (2026-10-07)

### Implemented

| File | Purpose |
|---|---|
| `build.rs` | Embeds `assets/icon.ico` as Win32 resource `1 ICON` into the exe. Windows-only: returns immediately when `CARGO_CFG_TARGET_OS != "windows"`. Writes `nameshot.rc` to `OUT_DIR`, compiles it with `x86_64-w64-mingw32-windres`/`windres` (GNU) or `rc.exe` (MSVC), then links the object via `cargo:rustc-link-arg-bins`. Missing icon or missing compiler → one `cargo:warning`, build continues (default icon kept). Emits `cargo:rerun-if-changed=assets/icon.ico` so replacing the file alone re-triggers the build. No new crate dependencies (pure `std`). |
| `assets/icon.ico` | **TEMPORARY placeholder** — 4 PNG-compressed frames (16/32/48/256 px, 32-bit alpha) generated from the draft `assets/icon.png`. To be replaced with the final `.ico` at the same path; no code changes needed afterwards. |
| `assets/make_placeholder_icon.ps1` | Regenerates `icon.ico` from a PNG (System.Drawing high-quality downscale → ICO container). Prints a TEMPORARY notice when run. |
| `README.md` | Project overview, verified-status table, feature list as actually implemented, build instructions (with the exact PATH line used on this machine), icon replacement procedure, hotkey caveat, layout. |

### Actually run and observed (2026-10-07)

Build-script guard, exercised by compiling `build.rs` standalone (`rustc --edition 2024`,
exit 0) and running the binary under controlled environments:

| Scenario | Result |
|---|---|
| `CARGO_CFG_TARGET_OS=unix`, `OUT_DIR` deliberately invalid | exit **0**, no output (early return; no `OUT_DIR` access) |
| `CARGO_CFG_TARGET_OS` unset | exit **0**, no output |
| `=windows`, valid `OUT_DIR`, `windres` **not** on PATH | exit **0**, prints `cargo:warning=no resource compiler …` |
| real cargo build (windres on PATH) | exit **0**, artifacts produced |

Icon embedding verified end-to-end on the real build:

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.77s   (exit 0)
OUT_DIR\nameshot.rc       25 B    (generated: 1 ICON "assets/icon.ico")
OUT_DIR\nameshot_icon.o    50 060 B (windres COFF object, linked into the exe)
target\debug\nameshot.exe  20 589 797 B (rebuilt 14:13:19)
```

`[System.Drawing.Icon]::ExtractAssociatedIcon('target\debug\nameshot.exe')` returns
**the placeholder artwork** (dark rounded square, blue selection brackets, white pen) —
saved as `p4_exe_icon.png` and inspected visually: the resource is inside the executable,
not just on disk. (Windows picks the first icon group for Explorer/taskbar/Alt-Tab; the
tray icon is drawn programmatically and unchanged.)

Placeholder ICO self-check: `System.Drawing.Icon` loads it (32×32 default), header
`type=1 count=4`, each frame's first bytes are the PNG signature `89 50 4E 47`.

### Checks run (exact results)

| Command | Result |
|---|---|
| `rustfmt --edition 2024 --check build.rs` | **exit 0** (new file is rustfmt-clean) |
| `cargo fmt --all -- --check` | **exit 1**: 169 diff hunks in **25 pre-existing files** — the codebase was written without rustfmt and was never formatted (true before Phase 4; verified the count before/after: 26 files initially, `build.rs` fixed, 25 remain). **Deliberately not reformatting** the verified Phase 2/3 sources in a Phase-4-only change; a whole-repo `cargo fmt` is a candidate for a later phase with a full re-test. |
| `cargo check --all-targets` | **exit 0**, `Finished dev profile … in 3.51s`, 0 errors / 0 warnings |
| `cargo test` | **exit 0**: 46 unit + 4 integration passed, 0 failed (`Finished test profile … in 10.45s`) |
| `cargo build` | **exit 0**, 10.77s, exe rebuilt with embedded icon (above) |

### Notes

- Privacy claim checked: source search for `Ws2_32|WinHttp|WinInet|InternetOpen|URLMon|
  Win32_Networking|socket` → **no matches**; `Cargo.toml` still has exactly one dependency
  (`windows`). This backs the README's "no network code" statement.
- README documents the placeholder explicitly: path `assets/icon.ico`, expected format
  (multi-size `.ico`, 16/32/48/256, 32-bit with alpha), regeneration command, and the
  build-script behaviour when the icon or a resource compiler is absent.
- Known non-blocking item carried over: repo-wide rustfmt divergence (see table above).

### Exit criteria (Phase 4)

- [x] Icon embedded and visually verified inside `nameshot.exe`; replace-later workflow
      needs only a new `assets/icon.ico` (+ `cargo build`), zero code changes.
- [x] Resource embedding guarded: non-Windows run of the build script exits 0 without
      touching any Windows tool; missing windres/rc.exe degrades to a warning.
- [x] README.md written from actual state; no claim beyond what Phases 0–3 verified.
- [x] Checks executed and reported exactly: fmt (new file clean; repo-wide pre-existing
      divergence documented), check 0/0, test 50/0, build 0.

---

## Plan reconciliation (2026-10-07) — numbering and scope authority

### Which plan is authoritative

The **original phased execution plan** (heading `# Phased execution plan`, Phases 0–6)
issued in the project's first requirement message is **authoritative** for phase
numbering, scope, and acceptance criteria. On 2026-10-07 a numbering conflict was
reported: the redaction-test entry below was labelled "Phase 5", but the original plan
reserves Phase 5 for *Settings, polish, and performance*. The plan had not been persisted
in the repository; on 2026-10-07 it was recovered **verbatim** from the opencode session
store (`opencode.db`) and is summarised below. From this point on, phase numbers in this
log refer to the original plan unless an entry is explicitly labelled "executed phase".

Two numbering schemes exist and both are legitimate:

| Scheme | Where | Meaning |
|---|---|---|
| Original plan (authoritative) | first requirement message; summarised below | Phases 0–6 as contracted |
| Executed-phase numbering | this BUILD_LOG, Phases 0–4 / 4.5 | chronological batches actually built |

They diverged early: execution was re-batched (the OS-free core and the Windows UI were
built as executed Phases 1–2 instead of the original per-feature Phases 1–4, and an
icon/README batch had no original counterpart). The executed numbering is kept as the
historical record of *when* work happened; **what a phase must achieve is defined only by
the original plan**.

### Original plan ↔ this log

| Original plan phase | This log | State |
|---|---|---|
| 0 — Repository and toolchain reconnaissance | Phase 0 | done |
| 1 — Minimal native shell and tray lifecycle | executed Phases 1–3 (built together, live-verified in Phase 3) | done (both criteria met, see below) |
| 2 — Screen capture and selection overlay | executed Phases 1–3 (+ Esc-cancel live in Phase 6) | done; doc gap closed in Phase 6, multi-monitor still untested |
| 3 — Native editor and annotations | executed Phases 1–3 (+ mosaic/solid live in Phase 6) | implemented + unit-tested; per-tool live pass still open |
| 4 — Privacy redaction and output | executed Phases 1–3 + **Phase 4.5** (its item 2) + Phase 6 | tests + copy/save + live redaction verified; `Ctrl+Z`/`Ctrl+Y` still unverified |
| (no counterpart) — icon embedding + README | Phase 4 | done (README also required by the plan) |
| **5 — Settings, polish, and performance** | **Phase 5 (original numbering)** | done — live-verified 2026-10-07 |
| **6 — Verification, packaging, and delivery** | **Phase 6 (original numbering)** | done — live-verified 2026-10-07, 31/31 checks |

### Original acceptance criteria — final status

**Still open (the only two):** the live per-tool editor pass with undo/redo (Original
Phase 3) and the live `Ctrl+Z`/`Ctrl+Y` check (Original Phase 4) — both re-scoped with
their real current state below. Everything else was completed and ticked across Phases
4.5–6; nothing from the test-only Phase 4.5 was counted toward other phases' criteria.

**Original Phase 1**
- [x] "release build starts" — **complete as of Phase 5** (`cargo build --release` exit 0,
      exe 2,084,859 B; the live Phase 5 verification ran exclusively against it).
- [x] "idles without polling" — blocking `GetMessage` loop, and Phase 5 measured idle at
      0.000 CPU seconds over 20 s (~0% of one core), 0 sockets.

**Original Phase 2**
- [x] "document any unverified multi-monitor/DPI scenarios" — **complete as of Phase 6**:
      the machine is single-monitor 1920×1200; multi-monitor offsets and mixed-DPI
      desktops remain untested, and the gap is now recorded in README ("Known
      limitations" + the last QA-checklist line) and in the Phase 6 entry below.

**Original Phase 3**
- [ ] "each tool visibly works" / "undo/redo is correct" live in the editor window —
      Phase 3 verified capture→editor→copy/save only; **Phase 6 added live passes of the
      mosaic and solid tools** (property-verified output, two full sessions) plus live
      toolbar navigation (Tab/Enter). Still not live-exercised: pen/line/arrow/rect/
      box/text/highlight and `Ctrl+Z`/`Ctrl+Y` (undo/redo is implemented at
      `src/editor/mod.rs:859/:862` with 3 dedicated unit tests in `annotate.rs`;
      rendering correctness of every tool is covered by unit/integration tests).

**Original Phase 4**
- [ ] Live verification of `Ctrl+Z`/`Ctrl+Y` — both are implemented
      (`src/editor/mod.rs:859`, `:862`); Phase 3 exercised only `Ctrl+C`/`Ctrl+S`,
      Phase 6 added `Ctrl+C`/`Ctrl+S` in two more sessions (copy==save pixel-identical).
- [x] Item 2 — "automated tests proving the exported output does not preserve original
      pixels in redacted regions or hidden layers" — **complete as of Phase 4.5 below**
      (this is what that test-only phase delivered; it was never a Phase 5 criterion).

**Original Phase 5 — Settings, polish, and performance — complete as of Phase 5 below
(2026-10-07)**
1. [x] Versioned settings file; invalid/missing settings validated and recovered to safe
       defaults — live-verified: S1 persistence round-trip, S2 corrupt-file recovery
       (all 7 corrupt values fell back to safe defaults).
2. [x] Configurable hotkey and output folder/format; startup-on-login off unless the user
       enables it — audited (HKCU `Run` written only on explicit enable, default off)
       and live-verified end-to-end (change → persist → restart → read back).
3. [x] Polish dark/minimal interface, error messages, keyboard states, accessibility
       names — audited (no defects found) + new `src/win/a11y.rs` exposes
       `PROPID_ACC_NAME` on four controls, verified live via MSAA `accName`.
4. [x] Release build with measured idle CPU / private working set / peak capture-session
       memory, no benchmark loop inside the app, conditions recorded — done, exact
       numbers and conditions in the Phase 5 entry.
5. [x] Verify no network access, analytics, auto-upload, or hidden capture history —
       formalised in the Phase 5 entry (`cargo tree`, code-path audit, live socket
       counts in three states).
- Exit criteria — settings survive restart; defaults privacy-safe; idle process
  quiescent; reported measurements real and reproducible: **met, all four checked in
  the Phase 5 entry**.

**Original Phase 6 — Verification, packaging, and delivery — complete as of Phase 6 below
(2026-10-07)**
- [x] Item 1 — formatting/lint/all tests/release build; fix warnings that indicate
      correctness, safety, or resource problems: all five gates re-run fresh (table in
      the Phase 6 entry). Clippy shows only style/docs-level findings (classified, no
      correctness/safety/resource warnings); `cargo fmt --check` divergence (169 blocks
      in 25 pre-existing files) recorded instead of rewriting formatted code — both per
      plan wording, nothing to fix.
- [x] Item 2 — review unsafe code, resource cleanup, hotkey conflicts, clipboard
      ownership, DPI arithmetic, redaction export paths: findings table in the Phase 6
      entry (no defects found; every export path funnels through flatten→redact).
- [x] Item 3 — complete README, ARCHITECTURE, BUILD_LOG + concise manual QA checklist:
      README gained Running/package, measured resources, the 10-item QA checklist,
      Known limitations and Troubleshooting; ARCHITECTURE module table drift fixed;
      BUILD_LOG updated (this entry + reconciliation).
- [x] Item 4 — separate passed automated/core checks from manual checks still required:
      Windows-specific execution was available, so every runnable check was run live;
      the remaining unverified items (multi-monitor, mixed-DPI, DRM, live per-tool
      editor pass) are listed explicitly in the Phase 6 entry and README.
- [x] Item 5 — honest final summary: Phase 6 entry §7 below; no perfect-optimization or
      universal-compatibility claims.
- [x] The 8-box final acceptance checklist — ticked with per-box evidence in the Phase 6
      entry §7 below (7 of 8 fully met; the editor box stays open because the live
      per-tool/undo-redo pass of original Phase 3 was never done).

### PrintScreen conflict and fallback (kept prominent, unchanged)

Windows/explorer may already hold bare `PrintScreen` (snipping hook): `RegisterHotKey`
fails with error 1409 and NameShot reports *hotkey unavailable* while staying running.
**Fallbacks: capture from the tray menu, or set `Alt+PrintScreen` (or any other hotkey)
in Settings.** Also documented in README ("Hotkey note").

---

## Phase 4.5 — Redaction-correctness tests (2026-10-07, executed-phase numbering)

Fulfils **original plan Phase 4, item 2** (not original Phase 5 — see the Plan
reconciliation above). Renamed from "Phase 5" on 2026-10-07 when the numbering conflict
was found.

### Scope confirmation (checked before any work)

`README.md` and `docs/BUILD_LOG.md` define no phase after Phase 4. The only phase-like
work item documented anywhere in the repository is the redaction test suite promised by
`docs/ARCHITECTURE.md` ("Redaction correctness model", point 4): a file `tests/redaction.rs`
that "proves this on synthetic images" with four stated assertions — **but the file did not
exist** (`tests/` contained only `encode_roundtrip.rs`). Confirmed with the user via a
question (options: implement that file / restate the original plan / release-build phase);
the confirmed scope is exactly `tests/redaction.rs` below. No other work was invented; the
temporary icon and the pre-existing (unformatted) sources were not touched.

### Implemented

`tests/redaction.rs` — one integration test per ARCHITECTURE assertion:

| Test | Acceptance criterion (`docs/ARCHITECTURE.md`) |
|---|---|
| `mosaic_output_equals_block_means_and_contains_no_original_values` | "mosaic output equals block averages and contains no original per-pixel values" |
| `solid_output_is_fully_opaque` | "solid output is fully opaque" |
| `flatten_differs_from_source_only_inside_redaction_regions` | "flattened output differs from source only inside redaction regions" |
| `encode_decode_roundtrip_equals_flattened_buffer` | "encode → decode round-trip equals the flattened buffer byte-for-byte" |

Design: the synthetic source has **every colour channel even**, arranged per 4×4 cell as
`base, base+2, …, base+30`, so each cell mean is `base + 15` (odd) — a redacted value can
never coincide with an original one. The mosaic test asserts both per-pixel equality with
the independently computed mean (same rounding as `redact::pixelate`: `(sum + n/2)/n`) and
an **empty intersection** between original and output RGB value sets. The solid test uses a
translucent source (alpha 17) and translucent colour (alpha 42) to prove alpha is forced to
255 while the outside region stays untouched. The round-trip test exercises the editor's
real save path (`encode_to_file` PNG → `decode_file`, COM initialized as in
`tests/encode_roundtrip.rs`) over a mixed annotation list (rect + pen + mosaic + solid).

### Actually run and observed (2026-10-07)

| Command | Result |
|---|---|
| `rustfmt --edition 2024 --check tests/redaction.rs` | **exit 0** (file formatted with rustfmt; no other file reformatted) |
| `cargo check --all-targets` | **exit 0** |
| `cargo test` | **exit 0 — 54 passed, 0 failed**: 46 unit + 4 `encode_roundtrip` + 4 `redaction` (+ 0 doc); the new suite passed on its first execution |

### Exit criteria (Phase 4.5)

- [x] All four ARCHITECTURE assertions exist as executable tests and pass.
- [x] Phase 3/4 outputs, temporary icon, and pre-existing sources preserved unchanged
      (only `tests/redaction.rs` added; `README.md` status row/test count/layout line
      updated so documented numbers stay true; this entry appended).
- [x] Exact results recorded; no issues found in the code under test.
- [ ] Carried over unchanged (out of scope): repo-wide rustfmt divergence (25 pre-existing
      files), bare `PrintScreen` registration blocked by explorer's snipping hook.

---

## Phase 5 - Settings, polish, and performance (2026-10-07, original plan numbering)

Original plan items: (1) versioned local settings file with invalid/missing recovery,
(2) configurable hotkey/output folder/format, startup-on-login off unless explicitly
enabled, (3) polish interface/error messages/keyboard states/accessibility names,
(4) release build + measured idle CPU/private working set and peak capture-session
memory with recorded conditions and no benchmark loop inside the app, (5) verify no
network/analytics/auto-upload/hidden capture history. Exit criteria: settings survive
restart, defaults privacy-safe, idle quiescent, measurements real and reproducible.

### 1-2. Settings persistence, recovery, launch-at-login (audited + live-verified)

Items 1-2 were implemented in Phase 2; this phase audited and live-verified them.

- `src/config.rs`: versioned INI (`version=1`), tolerant parse (unknown keys, out-of-range
  values, wrong types, foreign version all fall back to safe defaults), atomic save
  (`write tmp + rename`), 8 unit tests, and it stores **no pixel data** ever.
- `src/hotkey.rs` + Settings window: hotkey and output folder/format configurable;
  `launch_at_login` written only to `HKCU\...\Run` via `src/os.rs` (`startup_entry`),
  default **off**, with the visible hint "Off by default. Never overwrites an existing file."
- Live `S1` (persistence round-trip) and `S2` (corrupt-file recovery): see table below.

### 3. Polish and accessibility names (implemented this phase)

- Error messages (`src/strings.rs`) and keyboard states (editor `on_key`: Ctrl+C/S/Z/Y,
  Tab cycles toolbar focus, Esc discards pending text; toolbar is a custom-drawn strip,
  so no native focus states apply) audited - no defects found.
- **New `src/win/a11y.rs`**: `set_name(hwnd, name)` uses `IAccPropServices`
  (`CLSID_AccPropServices`, `SetHwndPropStr`, `PROPID_ACC_NAME`) on **both** object ids
  (`OBJID_CLIENT` and `OBJID_WINDOW`), best-effort (`let _ =`, never blocks the UI).
  Cargo feature `Win32_UI_Accessibility` added. Call sites: Settings format combo
  ("Image format"), JPEG quality edit ("JPEG quality"), save-folder edit ("Save folder"),
  editor inline text input (`s().text_input_name` = "Text input", new `strings.rs` field).
- **Verified via MSAA** (`AccessibleObjectFromWindow` -> `accName`), which is the API the
  property-services override feeds: all four controls return exactly the set names
  (live run, see table). The .NET UIA client on this OS build reports the control's text
  through its native UIA provider instead of the `PROPID_ACC_NAME` override (observed on
  ComboBox/Edit alike, also with `OBJID_WINDOW` set) - a Windows UIA/MSAA interop quirk,
  not an application defect; every control additionally has a visible text label, so both
  client families receive a name. Live assertions therefore read `accName`.

### 4. Release build and measurements (this phase)

- `cargo build --release`: **exit 0**; `target\release\nameshot.exe` = **2,084,859 bytes**,
  built 2026-10-07 18:47:38. No benchmark loop or measurement code exists in the app -
  all numbers come from an external script reading OS process counters.
- Conditions: Windows 10 IoT Enterprise LTSC build 19044 x64, 1920x1200 single monitor,
  interactive GUI session, release exe, app driven via posted/sent messages only
  (no input simulation); idle measured after 8 s warm-up over a 20 s window; memory from
  `PrivateMemorySize64` / `PeakWorkingSet64`; sockets from `Get-NetTCPConnection` /
  `Get-NetUDPEndpoint` filtered by PID.

| State | Private working set | Peak working set | CPU over window | TCP/UDP sockets |
|---|---|---|---|---|
| idle (tray) | 1.6 MB | 8 MB | 0.000 s over 20 s (~0% of one core) | 0 / 0 |
| capture session (overlay) | 19.7 MB | 37.2 MB | n/a (short session) | 0 / 0 |
| editor session | 7.9 MB | 37.2 MB | n/a (short session) | 0 / 0 |
| after session exit | 2.3 MB | 37.2 MB (lifetime peak) | n/a | 0 / 0 |

### 5. No network, no analytics, no hidden history (this phase)

- Dependencies: `cargo tree` -> only `windows 0.62.2` and its sub-crates; no HTTP/socket
  or telemetry crate exists in the graph.
- Code paths: no `connect`/`send`/URL usage in `src/`; the only file writes in the whole
  app are the panic log (`main.rs`) and the settings save (`config.rs::save`).
  Captures are written **only** on explicit Save/Quick save, timestamped, never
  overwritten (existing files get a `_1`, `_2`, ... suffix) - there is no hidden history;
  quick save additionally surfaces as a normal visible file in the chosen folder.
- Runtime: 0 TCP and 0 UDP endpoints owned by the process in idle, overlay, and editor
  states (live, three measurements).

### Actually run and observed (2026-10-07)

| Command | Result |
|---|---|
| `cargo build --release` | **exit 0** (exe 2,084,859 B, 2026-10-07 18:47:38) |
| `cargo check --all-targets` | **exit 0**, no warnings |
| `cargo test` | **exit 0 - 54 passed, 0 failed** (46 unit + 4 encode + 4 redaction) |
| `rustfmt --edition 2024 --check src/win/a11y.rs` | **exit 0** (new file formatted; no repo-wide rewrite) |
| live `ns_p5.ps1` against the release exe | **26/26 PASS** |

Live results (S1 persistence / S2 recovery / S3 measurement + a11y):

- **S1**: OK wrote `format=bmp`, `quick_save=1` to disk; app exited cleanly; after restart
  Settings reopened and showed `combo=2 (BMP)`, `quick=1`, `quality=90`, `hotkey=Insert`;
  clean exit.
- **S2**: deliberately corrupt INI (`version=99`, `hotkey=banana`, `format=tiff`,
  `quick_save=banana`, `launch_at_login=yes`, `jpeg_quality=999`, unknown `telemetry=on`)
  -> recovered to `PNG`, quick off, startup off, `quality=90`, `hotkey=PrintScreen`,
  empty folder; startup box "Capture hotkey unavailable" shown for the conflicted default
  PrintScreen hotkey (the documented fallback path); clean exit.
- **S3**: measurements as in the table above; editor inline text input reports
  `accName=[Text input]`.
- **Restore**: `settings.ini` restored byte-identical to the pre-test baseline
  (hash match); no stray capture files in Pictures; no leftover process.

### Findings during verification (test tooling only - no application defects)

- The tray menu gains an item while `quick_save=1`, shifting menu navigation: opening
  Settings needs 3x Down instead of 2x (an early run pressed Quick save instead and left
  one stray BMP, which was deleted; it also proved Quick save's no-overwrite naming live).
- `GetWindowTextW`/`GetDlgItemTextW` return 0 for edit controls owned by another process;
  `SendMessage(WM_GETTEXT)` returns the real text - test reads use the latter.
- This machine's UIA client reports `ControlType=Pane` for all bridged Win32 controls
  (Notepad's Edit behaves identically) - client quirk, assertions key on `AutomationId`
  and MSAA `accName` instead.

### Exit criteria (Phase 5, original plan)

- [x] Settings survive restart (S1 round-trip).
- [x] Defaults remain privacy-safe (S2 recovery; launch-at-login off; no pixel data in
      the settings file).
- [x] Idle process is quiescent (0 CPU seconds over 20 s, 0 sockets).
- [x] Reported measurements are real and reproducible (external script, conditions and
      exact numbers recorded above; no in-app benchmark).

---

## Phase 6 — Verification, packaging, and delivery (2026-10-07, original plan numbering)

Original plan items: (1) run formatting, linting, all tests, release builds and fix
warnings that indicate correctness/safety/resource problems; (2) review unsafe code,
resource cleanup, hotkey conflicts, clipboard ownership, DPI arithmetic, redaction
export paths; (3) complete README/ARCHITECTURE/BUILD_LOG + a concise manual QA
checklist; (4) separate passed automated/core checks from manual checks still required;
(5) honest final summary — plus the 8-box final acceptance checklist. Windows-specific
execution **was** available on this machine, so every runnable check was run live.

### 1. Release gates (all re-run fresh in this phase)

| Command | Result |
|---|---|
| `cargo build --release` | **exit 0** (4.71 s, cached) — `target\release\nameshot.exe`, 2,084,859 B, built 2026-10-07 18:47:38, unchanged by this phase (no source edits) |
| `cargo test` | **exit 0 — 54 passed, 0 failed** (46 unit + 4 encode + 4 redaction) |
| `cargo check --all-targets` | **exit 0** — 0 errors, 0 warnings |
| `cargo clippy --all-targets` | **exit 0** — `nameshot (lib)` 35 warnings (19 auto-fixable), `nameshot (lib test)` 41 warnings (35 duplicates); 17 distinct warning texts, all style/docs/complexity (see classification below) → nothing matching "correctness, safety, or resource problems" → **no fixes required** |
| `cargo fmt --check` | **exit 1** — 169 diff blocks in 25 pre-existing files; recorded, not reformatted (repo convention: no repo-wide rustfmt rewrite). `src/win/a11y.rs` and `tests/redaction.rs` are individually rustfmt-clean (exit 0), and all files added/edited in later phases stay formatted |

Clippy classification (artifact: `%TEMP%\opencode\p6_clippy.txt`): the 17 distinct
warning texts are `too_many_arguments` ×2 (8/7 and 9/7), `chunks_exact`/`chunks_exact_mut`
with constant size ×2, `type_complexity`, `missing_safety_doc`, an unnecessary same-type
pointer cast, `collapsible_if`, `if_same_then_else`, an unneeded `Some`/`?` combination,
elidable explicit lifetime `'a`, index-only loop variable, `manual_div_ceil`,
`derivable_impls`, immediately-dereferenced reference, `field_reassign_with_default`,
and a no-effect operation. The only unsafe-adjacent two — the pointer cast (no behavioural
effect) and the missing `# Safety` doc section — were reviewed line-by-line and are
benign: all `unsafe` is confined to `win/*`, `capture.rs`, `app.rs` (wndproc/plumbing),
always behind RAII guards, and the OS-free core contains none. Every message is
style/docs/complexity; there are **no correctness, safety, or resource warnings to fix**,
and running `cargo fmt` repo-wide would rewrite 25 untouched files, so it is recorded
instead (per plan wording).

### 2. Production-path review (plan item 2) — no defects found

| Area | Finding |
|---|---|
| unsafe code | Only in `win/*`, `capture.rs`, `app.rs` (wndproc/plumbing); every GDI object/DC/clipboard/hotkey ownership is RAII (`win/gdi.rs`, `ClipGuard`, unregister on exit). OS-free core has zero `unsafe`. |
| resource cleanup | Sessions own buffers; Esc/close/drop frees them (overlay, editor, capture). Phase 5 + 6 live runs both end "no leftover process/files". |
| hotkey conflicts | `RegisterHotKey` error 1409 handled non-fatally (`hotkey_ok=false`, clear message, Settings rebind, tray fallback); no re-registration polling loop anywhere. |
| clipboard ownership | `GlobalAlloc`'d DIB/PNG buffers handed to `SetClipboardData` (system owns on success, freed on failure); `CF_DIB` + registered `PNG` format; open/empty/close guarded. |
| DPI arithmetic | `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` before any window (`app.rs:426`); all coordinates physical pixels; overlay spans the virtual screen incl. negative coords. |
| redaction export paths | `do_copy` (`src/editor/mod.rs:1212`), `do_save` (:1231), `quick_save` (`src/app.rs:135`) all call `preview.render(items, None)` → `flatten::apply` → `redact::pixelate`/`solid`; the `preview_incremental_matches_full_flatten` unit test proves preview == full flatten. No other call site reaches the encoders or clipboard (grep of `encode_to_file`/`clipboard::set` call sites). `tests/redaction.rs` (4 tests) proves the flattened-only contract byte-for-byte. |

### 3. Live verification — `ns_p6.ps1`: **31/31 PASS, FAILED=0** (2026-10-07 20:40–20:41)

Single attempt per session, no retries; settings baseline hash-restored; artifact:
`%TEMP%\opencode\p6_run_output.txt` (condensed PASS record) + diagnostic PNGs
(`p6_diag_R*_*.png`, `p6_*_pre*.png`).

| Session | What was driven | Key results |
|---|---|---|
| O1 | hotkey → overlay → **`Esc` cancels the capture session** | overlay gone, app alive, no editor (this closes the last untested capture-path branch) |
| R1 (mosaic) | hotkey → selection drag (500,300)→(1400,850) → Enter → editor → Tab×7+Enter (mosaic) → drag (80,200)→(700,430) → **Ctrl+C then Ctrl+S** → save dialog accepted (Enter posted directly to the dialog hwnd) | copy vs saved PNG **pixel-identical (diffPixels = 0 / 496,451, 901×551)**; mosaic redaction present in both: 90 cells, per-cell dense diff vs pre-capture crop, adjacent-row identity 0.933, only 308 distinct values where the source has 1,910 |
| R2 (solid) | same flow, Tab×8+Enter (solid) | copy vs save **pixel-identical (0 / 496,451)**; solid region fully opaque + uniform: 132,090 px, bbox 595×222 at (61,103) — exactly the dragged rect in image coordinates |
| cleanup | polite `WM_CLOSE` exit after each session | no leftover capture files, `settings.ini` byte-identical to baseline (hash), no process, no `%TEMP%\nameshot_panic.txt` |

Redaction properties were checked against the **pre-capture screen crop**, so the
mosaic/solid assertions prove the exported pixels carry no original content — the
production output path (`do_copy`/`do_save` → flatten → redact → WIC/clipboard), not a
test-only path.

Test-tooling notes (no application defects): driving the editor required posting input
messages rather than synthesizing hardware input, because the machine's operator was
actively using the desktop during runs (physical Space key switches the drag into pan
via `key_down(VK_SPACE)`; another mouse button steals `SetCapture` via
`WM_CAPTURECHANGED`). Save/`Enter` was posted straight to the `#32770` dialog window to
be race-proof against focus changes. Every session verified `settings.ini` restoration
by hash.

### 4. Documentation completed (plan item 3)

- **README**: Phase 6 status row; new *Running* section (package location, extract-and-run,
  **no runtime prerequisites** — DLL import audit below); `cargo build --release` in the
  build commands; *Measured resources* table (Phase 5 numbers + conditions); **10-item
  manual QA checklist**; *Known limitations* (single-monitor machine, multi-monitor/
  mixed-DPI untested, DRM/secure desktop, placeholder icon, unsigned, mosaic ≠ anonymity);
  *Troubleshooting* (hotkey conflict, panic log, paste formats, start-at-login);
  project-layout updated (`a11y.rs`, `os.rs`, `dist/`).
- **ARCHITECTURE**: module-table drift fixed — `main.rs` row now describes only the entry
  point/panic hook, `app.rs` row credits the OS gate/DPI/COM/message loop actually in
  `app.rs`+`os.rs`, `capture/mod.rs`+`GetDIBits` corrected to `capture.rs`+`BitBlt`/`CreateDIBSection`,
  added missing rows (`hotkey.rs`, `output.rs`, `win/a11y.rs`, `settings_ui.rs`), and the
  portability section's `capture/` path corrected.
- **BUILD_LOG**: this entry; Plan reconciliation updated (Phase 6 row, Phase 2 doc-gap
  criterion ticked, Phase 3/4 criteria re-scoped to their real current state, Phase 6
  five items + 8-box checklist ticked with evidence).

### 5. Passed automated/core vs. still manual/unverified (plan item 4)

**Passed and live-verified on this machine** (phases 3, 5, 6): tray lifecycle, hotkey +
conflict path, overlay open/Esc-cancel/drag-select, capture→editor→copy→save (Phase 3
baseline; Phase 6 repeated it with mosaic + solid property checks), settings persistence
+ corruption recovery, accessibility names via MSAA, idle/perf/network measurements,
release gates (54 tests, 0 warnings, clippy/fmt classified).

**Still unverified / manual-only (honestly recorded, none of it claimed):**

1. **Multi-monitor coordinates and mixed-DPI desktops** — the machine has one
   1920×1200 monitor. Design reviewed (virtual-screen bounds, negative coords,
   `PER_MONITOR_AWARE_V2`), never live-tested. Documented in README *Known limitations*
   and the QA checklist.
2. **Live per-tool editor pass** for pen/line/arrow/rect/box/highlight/text and live
   `Ctrl+Z`/`Ctrl+Y` — implemented and unit/integration-tested, but never driven
   interactively end-to-end (original Phase 3/4 criteria, still open above).
3. **DRM/protected content and secure desktop (UAC)** captures — inherent GDI limits,
   not tested.
4. **Code signing / installer** — explicitly out of plan scope; the package carries no
   signing or installer guarantees.

### 6. Packaging (plan title: "packaging and delivery")

- Artifact: **`dist\NameShot-0.1.0-win-x64.zip`** containing exactly:
  `nameshot.exe` (2,084,859 B), `README.md`, `docs\ARCHITECTURE.md`, `docs\BUILD_LOG.md`
  (the zip is built after all documentation edits, so the package carries this entry).
- Executable SHA-256 (both in the repo build and inside the zip):
  `199EDCEC9F5E93C1DD09194F900DAAFB27C0963253E10636ECD744A1F4201E1A`.
- Prerequisites audit (`objdump -p nameshot.exe`): imports only Windows system DLLs
  (KERNEL32/USER32/GDI32/comctl32/dwmapi/uxtheme/shell32/ole32/…) and the inbox
  Universal CRT (`api-ms-win-crt-*`, present since Windows 10). **No libgcc/libwinpthread/
  vcruntime, no WebView2, no .NET, no MSVC redistributable, nothing bundled or installed.**
  (WS2_32.dll appears only as a binding import; the Phase 5 audit found 0 sockets in all
  states and no network code paths.)
- Verification performed: extract zip to a temp dir → SHA-256 of the extracted exe
  **equal** to the repo build → launch the *extracted* copy → main window
  (`NameShot.Main`) found → polite `WM_CLOSE` → process exited (**smoke pass**).
  No installer, no code-signing claim.

### 7. Final acceptance checklist (original plan, verbatim boxes)

- [x] **Native Rust/Win32 app; no Electron, Tauri, WebView, Chromium, Node runtime, or
      background server.** — single dep `windows 0.62.2` (`cargo tree`), plain Win32
      windows + `GetMessage` loop; DLL audit shows no browser/runtime libraries.
- [x] **Tray lifecycle and configurable global capture hotkey work; hotkey conflict does
      not break the app.** — Phase 3 tray live, Phase 5 S1 hotkey rebind persists across
      restart, S2 showed the conflict message with the app still running; Phase 6 ran
      both sessions through the registered hotkey.
- [x] **Region capture, multi-monitor-aware coordinates, Escape cancellation, and
      minimal-resource idle message loop.** — capture/Esc/idle live-verified (Phase 3,
      O1 in Phase 6, idle 0 CPU-s/0 sockets in Phase 5); *multi-monitor coordinates:
      design-reviewed, single-monitor machine — recorded caveat, not live-tested.*
- [ ] **Dark, original, compact editor with core annotations and undo/redo.** — editor
      dark/compact/original with all nine tools implemented and unit/integration-tested;
      mosaic + solid + toolbar navigation live-verified with property checks (Phase 6),
      but no drawing tool and no undo/redo keystroke has ever been driven live end-to-end
      (Phase 3 opened the editor only for copy/save) → **remains the one open box**
      (original Phase 3/4 criteria above).
- [x] **Pixelate and solid redaction applied to the actual flattened copied/saved output;
      automated tests cover this.** — Phase 6: copy == save pixel-identical and redaction
      property-checked in both for both tools; `tests/redaction.rs` 4/4 passing; export
      path audit in §2.
- [x] **Copy and local save work; no default network requests or automatic upload.**
      — Phase 3 + Phase 6 live copy/save; Phase 5: no network crates, no network code,
      0 TCP/UDP endpoints in idle/overlay/editor.
- [x] **Resource use and test coverage are measured and described honestly.** — Phase 5
      measurement table (conditions recorded) + this phase's gates (54 tests, clippy/fmt
      classified with exact counts).
- [x] **Documentation provides reproducible build, run, and manual verification steps.**
      — README (commands, Running section, QA checklist, troubleshooting), ARCHITECTURE
      (module table now matches the code), this log (per-phase evidence).

### 8. Honest final summary (plan item 5)

NameShot was **implemented, tested, run, measured and packaged** on this machine: a
single 2.08 MB native Rust/Win32 exe (no runtime install) with tray + configurable
hotkey, virtual-screen region capture with Esc-cancel, a dark editor with nine tools
including mosaic/solid redaction, clipboard (`CF_DIB`+PNG) and native-dialog save with
PNG/JPEG/BMP, versioned crash-safe settings, accessibility names, and zero network code.
Evidence: 54 automated tests green, 0 compiler warnings, live phases 3/5/6 at
50+26+31 checks with only original-Phase-3-style gaps left (per-tool live editor pass,
multi-monitor/mixed-DPI), measurements as in the Phase 5 table, package verified by
hash + smoke run. **Not claimed:** perfect optimization, universal compatibility,
multi-monitor/DPI coverage, code signing, or an installer; two acceptance criteria
(original Phase 3 live per-tool pass and its `Ctrl+Z`/`Ctrl+Y` live check) remain open
and are listed as such in "Original acceptance criteria — final status" above.

### Exit criteria (Phase 6, original plan)

- [x] Formatting/lint/tests/release build run; only style-level lint findings remain
      (classified, nothing correctness/safety/resource-related to fix).
- [x] unsafe/resource/hotkey/clipboard/DPI/redaction review completed — no defects found.
- [x] README, ARCHITECTURE, BUILD_LOG completed + concise manual QA checklist added.
- [x] Passed automated vs. still-manual split documented (§5).
- [x] Honest final summary (§8) + 8-box acceptance checklist with evidence (§7).

## Phase A — Inline-editor corrective task, diagnosis (2026-10-08)

Scope: diagnose the two release-blocking UX problems that triggered this corrective
task — (1) capture → annotate flow interrupted by a separate editor window,
(2) screenshot flickers/blinks on tool selection/use — **before** implementing the
inline-editing redesign (Phase B). Live runs used the release build
(`target\release\nameshot.exe`), `PostMessageW`-only driving, passive
`CopyFromScreen` sampling, and `settings.ini` backup/restore verified by hash after
every run. **Caveat:** the operator actively used the desktop during the runs
(moving windows, wheel-zooming, drawing); several early observations were therefore
environment artifacts, listed under "Ruled out / corrected" below.

### Problem 1 — separate editor window interrupts the flow (LIVE + CODE)

- Code: `overlay.rs` `confirm()` :576-612 calls `DestroyWindow` (:609) and then
  `editor::open` (:611); `editor::open` (`editor/mod.rs:270-366`) creates a brand-new
  `NameShot.Editor` window (`CreateWindowExW` :336, `ShowWindow(SW_SHOW)` :366).
  The overlay is destroyed before the editor exists → a desktop gap is structurally
  unavoidable with this design.
- Live: run 2 window timeline — editor hwnd appears ≤ 12 ms after overlay death,
  but the window is **not visible until ~305 ms**, i.e. the desktop (and whatever the
  operator had focused) is exposed for 50–300+ ms between "overlay gone" and
  "editor painted". Hwnd-level gap measured in run 1: 1.8 ms (destroy → new hwnd).
- Verdict: **confirmed**. Fix direction for Phase B: no window transition at all —
  edit inside the existing overlay (`Phase::{Selecting, Editing}`).

### Problem 2 — flicker/blink on tool use (LIVE + CODE)

- Code: `editor/mod.rs` `paint()` :964-1068 paints **directly to the visible DC in
  phases** with no double buffering: `fill_solid(tb)` → `fill_solid(status)` →
  `fill_solid(canvas)` → `preview.render()` → `StretchDIBits` → border → status →
  toolbar. `WM_MOUSEMOVE` during a drag calls `invalidate_canvas` per move event
  (~60 Hz), so every drawing gesture re-runs the whole fill→render→blit pipeline
  on screen; `commit_any_text` / undo / redo / `WM_APP_STATUS` invalidate the whole
  client (single text-commit frame captured in run 1 showed background-only paint).
- Live (instrumented release build, run 3): blank-canvas frames (canvas region
  **exactly** the `CANVAS` color, 590,611 px = 973×607, status still correct) captured
  in 5 separate tool-click probes (`click_Mark` ×2 consecutive, `click_Mosaic`,
  `click_Redact`, `click_Pen` ×2, earlier runs `click_Line` ≥ 543 ms, `click_Box`,
  `fx2_text_commit_0`). An earlier sequence of consecutive blank frames during a pen
  drag matched a sustained ~60 Hz invalidation storm (paint phases observable for
  most of each frame's time).
- Decisive instrumented run: `paint()` logged `StretchDIBits` return, `dw/dh`,
  image-vs-canvas overlap, `zoom/view/canvas/rcPaint` on every anomaly, plus
  `TOOL` state snapshots on every tool command — **11/11 tool clicks with identical,
  correct state (`fit=true`, `zoom=1.0436`, `view=(-15.69,-15.33)`, canvas
  `(0,76,973,683)`), 0 blit failures, 0 skipped blits, 0 off-image cases** — while
  blank frames were still captured. Conclusion: the app never painted a
  fill-without-image state to completion; the screen showed the **intermediate
  phase of an in-progress repaint** (canvas `fill_solid` written, blit not yet run),
  sampled by the concurrent screen capture / DWM composition, and the missing image
  persists on screen until the next canvas-region invalidation (tb/status-only
  paints never refresh it) — i.e. exactly the reported "screenshot blinks when
  picking/using a tool".
- Verdict: **confirmed root cause = single-buffered, phased direct-to-screen painting
  amplified by per-mousemove full-canvas invalidation**. Fix direction for Phase B:
  retained offscreen backbuffer, compose fill+image+border+status+toolbar in it,
  present with a single `BitBlt` per paint; region-aware invalidation.

### Ruled out / corrected (with evidence)

- **GDI-object quota exhaustion** (candidate cause of intermittent blit failure):
  brush/pen lifecycles audited — `gdi.rs` `fill_solid` (create→FillRect→delete),
  `toolbar.rs` `Content::Dot` (select→Ellipse→restore→delete :326-341) are balanced;
  the instrumented run measured **0 blit failures** anyway. Not the cause.
- **Bad view/zoom state** (huge `view` offset blanking the canvas): instrumented
  state logs identical and correct at all 11 tool clicks; `clamp_view` (:195) sound.
  Not the cause.
- **Operator wheel-zoom false positives**: run-1 "anomalies" at cv≈498k were the
  image at 43% zoom (1.04×0.8⁴) from the operator zooming — not a defect.
- **Fresh-tray CPU regression**: `CPU_TRAY_FRESH_10s=0.000 s`; the earlier 2.234 s
  reading was polluted by test-driven activity. Not a defect.
- **Run-1 multi-second "editor missing"**: operator's other window was on top
  (z-order theft); the app-side gap is the 50–300 ms paint latency above.
- **`FLASH_DELTAS=0` in run 2**: test-harness scoping bug (function-local
  `$flashLog` shadow), fixed in run 3 with `$script:anoms`; run-3 flash counting is
  the trustworthy one.

### Instrumentation note (temporary, reverted)

A throwaway build added `dbg_log` anomaly lines in `paint()`/`command(Tool)`
writing to `paint_debug.log`. After the decisive run, `src\editor\mod.rs` was
restored from a byte-identical backup (verified: grep for `dbg_log|paint_debug`
across `src` → no matches), `paint_debug.log` deleted, and `cargo build --release`
re-run clean (exe timestamp 2026-10-08 11:41). The repository and release build
contain no instrumentation.

### Exit criteria (Phase A)

- [x] Problem 1 reproduced live with measured timing; code cause identified.
- [x] Problem 2 reproduced live (blank/fill-phase frames); code cause identified
      and decisive instrumented run distinguishes "never painted wrong" from
      "painted wrong".
- [x] Alternative hypotheses audited and ruled out with evidence.
- [x] Diagnosis recorded here separating live observations from code-level
      reasoning; false positives and harness bugs explicitly corrected.
- [x] Repository reverted to pre-instrumentation state; release build clean.

## Phase B — In-overlay editing, no separate window (2026-10-08)

### Implemented (code)

- `overlay::Phase::{Selecting, Editing}`: `confirm()` no longer destroys the
  overlay. It crops the hotkey-time capture (`ov.frame`) to the selection,
  builds `EditSession::new(cropped, RECT canvas, (vs.w, vs.h), secure)` and
  flips the same window into `Phase::Editing` — the HWND never changes
  (live-verified: same overlay hwnd across Enter), the desktop is never
  exposed, and the capture is never repeated.
- `Backbuf` (`MemDc` + `OwnedGdi<HBITMAP>` + previous-object restore in `Drop`,
  field order mem→bmp): `paint()` composes fill → image → border → status →
  toolbar **into the backbuffer, clipped to `ps.rcPaint`**, then presents with
  a single `BitBlt`. This removes the fill-without-image intermediate state
  from the screen (Phase A root cause 2).
- `EditSession::apply(cmd) -> Redraw::{Chrome,Document}`: Tool/Color/Width are
  `Chrome`-only — the canvas region is **never invalidated by a tool click**,
  so the frozen selection cannot blink (unit-tested:
  `color_and_width_selection_are_chrome_only`,
  `tool_changes_never_touch_the_document`).
- Text tool: child `EDIT` subclassed (`edit_proc`, `SUBCLASS_ID 0x454F`),
  accessible name via `win::a11y::set_name` (edit.rs:1079), Enter commits,
  Esc cancels, destroyed on WM_DESTROY.
- Close path: `try_close` = commit pending text → `dialog::confirm` only when
  `history.is_dirty()` (title "Discard capture?") → `DestroyWindow`.
  WM_CLOSE and `exit_app` route through the same dirty check. All dialog call
  sites drop the session borrow first (audit, Phase D).
- Keyboard (editing): Esc close, Enter commit/confirm, `Ctrl+Z/Y/C/S`,
  Tab focus cycle across the rail/strip.
- `restart()` (R, selecting phase) now recreates the backbuffer when the
  virtual screen size changes (Phase D finding, fixed in this phase's code).
- `editor::open` retired from the flow: **0 callers** (grep `editor::open(`),
  kept as code. The editor window cannot appear in the current flow (also
  live-observed: "editor window never appeared").

### Actually run and observed

- `cargo build --release` → **exit 0, 0 warnings** (final state).
- `cargo test --release` → **exit 0; 62 passed, 0 failed** (54 lib incl. 8
  `overlay::edit` tests + 4 + 4 integration).
- Live harness `ns_pb1.ps1` (PostMessage/keybd, no operator input required for
  the written assertions) iterated over 3 runs. Passing assertions in the last
  run: main+overlay open, **same overlay hwnd across Enter**, rail/strip drawn,
  frozen canvas image stable, line/pen commits produce pixel diffs, Copy puts a
  **real image on the clipboard**, dirty-Esc raises the discard dialog,
  **No keeps the overlay with canvas intact (diff=0)**, undo restores the base
  exactly, child EDIT created/destroyed, editor window never appears, app
  alive after overlay close, settings restored.
- The same run carried 7 unexplained artifacts (an extra committed line and a
  mosaic rect not in history; text typing dropped). They are recorded as
  **unresolved** — the run was contaminated by concurrent operator input
  (foreground-lock stolen the keystrokes at least once), so it cannot
  distinguish app defects from test interference. A hands-off re-run is
  **deferred at the user's request** ("Later — I'll tell you when to run").

### Exit criteria (Phase B)

- [x] Enter keeps one continuous overlay window; no desktop exposure code path.
- [x] Tool/color/width clicks cannot invalidate the canvas (unit-tested).
- [x] Build clean, all tests green.
- [ ] Clean-machine live re-run of `ns_pb1.ps1` with all assertions passing
      (deferred; harness ready).

## Phase C — Overlay UI polish (2026-10-08)

### Implemented (code)

- **Hover tooltips** rendered in the status chip: hovering any rail/strip
  button replaces the chip text with `Label: hint` (e.g. `Mosaic: Pixelate a
  region`, `Undo: Undo last change (Ctrl+Z)`, `Color: Red`,
  `Stroke width: 4 px`); leaving the button restores the transient message or
  the image dimensions. Implementation: `strings.tip_*` + `color_names`,
  `tip_for(Cmd)`, `hover_tip` field, chip precedence
  (hover > message > dims), status-rect-only invalidation on change — **no
  floating tooltip windows, no timers, no extra repaints** (constraint
  compliant).
- Shortcuts are advertised only where wired (`Ctrl+Z/Y/C/S`, `Esc`) — enforced
  by the new unit test `tips_cover_every_button_with_shortcuts_only_where_wired`.
- Pre-existing chrome verified as already covering the rest of the plan:
  hover/selected/disabled/focus-ring states via `toolbar::paint`, all nine
  tools (mosaic + solid included) on the rail, accessibility name on the text
  `EDIT`. Deliberate scope decisions: no icon redesign (text labels are
  feature parity with the previous editor toolbar) and no pressed state
  (the retired editor window had none either). Zoom/pan exists only in the
  retired editor window — now documented as a known limitation in README.

### Actually run and observed

- `cargo build --release` → exit 0; `cargo test --release` → 62 passed
  (incl. the new tip test); `cargo clippy` → exit 0, baseline unchanged.
- Visual appearance of the new chip text: not yet eyeballed (live re-run
  deferred with Phase B).

## Phase D — Constraint audit (2026-10-08)

Every plan constraint re-checked against the final code; grep evidence where
noted, manual reading where a grep cannot prove absence:

- [x] **No timers/polling/sleeps** — grep `SetTimer|WM_TIMER|Sleep(|thread::sleep`
      over `src` → 0 matches.
- [x] **No layered/composited windows** — grep `WS_EX_COMPOSITED|WS_EX_LAYERED|
      UpdateLayeredWindow` → 0 matches.
- [x] **No instrumentation residue** — grep `dbg_log|paint_debug|eprintln!|dbg!(|
      println!(` → 0 matches.
- [x] **Single-buffered painting removed** — every paint goes through `Backbuf`
      (compose → one `BitBlt`); fallback path only if backbuf creation failed.
- [x] **No session borrow held across message-pumping calls** — read every
      dialog/clipboard/capture call site: `command()` dispatches Copy/Save/Close
      before borrowing; `do_copy`/`do_save` extract the frame in a scoped borrow
      and call `copy_image`/`save_image`/`error_box` after it is dropped;
      `try_close`, WM_CLOSE, `confirm()`, `restart()`, `quick_save()`,
      `exit_app()` all drop borrows first (e.g. `confirm()` `drop(b)` before
      `warn_box`, mod.rs:772/785/792).
- [x] **Region-limited invalidation** — editing-phase invalidations are rect-scoped
      (`invalidate_canvas/bar/status/btn`, `inv()`); full-window invalidates exist
      only at phase transitions and `restart()` (bounded events, not mousemove).
      Selecting-phase drags use `invalidate_region` (union of old/new band).
- [x] **GDI lifecycles** — `Backbuf::Drop` restores the previous object before
      `DeleteDC`/`DeleteObject`; `MemDc`/`OwnedGdi`/`SelectedObject` RAII;
      `SelectObject` restore pairs verified at every ad-hoc site.
- [x] **`paint_face` shared** with `settings_ui` (settings_ui.rs:509 →
      toolbar.rs:359) — untouched.
- [x] **No nested app message loops** — dialogs are system modal boxes only.
- [x] **`editor::open` retired** — 0 callers (grep).
- [x] **Backbuffer sizing on restart** — fixed in Phase B (was stale after a
      virtual-screen size change).

## Phase E — Automated gates (2026-10-08)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Release build | `cargo build --release` | 0 | 0 warnings |
| Unit + integration tests | `cargo test --release` | 0 | **62 passed, 0 failed** (54 lib + 4 + 4) |
| Clippy | `cargo clippy --release --all-targets` | 0 | 35 lib warnings = pre-existing baseline (41 lib-test incl. 35 duplicates) |
| Format check | `cargo fmt --check` | 1 | pre-existing, repo-wide (file was never rustfmt-formatted; no repo-wide fmt run per constraints) |
| Instrumentation / timers / layered grep | see Phase D | — | 0 matches each |
| Docs consistency | README flow text + QA checklist + status row | — | updated in Phase F |

**Live QA (Phase E, second half): not run in this phase.** The rerun of
`ns_pb1.ps1` is deferred at the user's explicit request; nothing in this log
claims live passage for Phases B–C beyond the partially-passing contaminated
run recorded in Phase B.

## Phase F — Documentation (2026-10-08)

- **README.md**: intro and Capture/Editing feature bullets rewritten for the
  in-overlay flow (Enter → edit in place, chip tooltips, shortcuts), QA
  checklist lines updated, zoom/pan added to *Known limitations*, status table
  gained the corrective-task row, project-layout note that the separate editor
  window exists but is no longer opened.
- **BUILD_LOG.md**: Phases B–F recorded (this section).
- **Packaging: untouched.** `dist\NameShot-0.1.0-win-x64.zip` and its hashes
  come from the previous delivery phase and were deliberately not rebuilt;
  the zip therefore still contains the pre-Phase-B exe. A re-package decision
  belongs to the user's review step, not to this corrective task.

### Exit criteria (Phases B–F, this corrective task)

- [x] Enter no longer destroys/recreates the window (code + partial live proof).
- [x] Tool clicks cannot repaint the canvas (unit-tested; single BitBlt present).
- [x] All automated gates green (table above).
- [x] Constraint audit clean (Phase D).
- [x] Docs reflect the shipped behaviour.
- [ ] **Live clean-machine QA re-run** (deferred by user; harness ready).
- [ ] **Re-package** (explicitly out of scope until user review).

## Phase G - Text-edit focus-loss survival + fail-closed QA harness (2026-10-08)

Driver: the first full live QA run (Phase B/E rerun, 2026-10-08 14:13) failed at
the text step (`typed text reaches EDIT ('')`), and the follow-up diagnostic run
localized the cause: an external window stole the foreground while the text box
was focused, the child EDIT received `WM_KILLFOCUS`, and `edit_proc` reacted by
commiting-and-destroying the box; focus later returned to NameShot, so the QA
foreground guards passed while the EDIT was already gone and the typed keys were
lost. A second, independent defect was found in the QA harness itself: PowerShell
guards that returned @(log-string, $false) made every `if` clause evaluate TRUE
(non-empty array), so keystrokes were injected despite a failed guard.

### Production changes (src/overlay/edit.rs, src/overlay/mod.rs, src/app.rs)

- `EditSession::on_text_focus_lost()` (deliberately empty policy hook),
  `EditSession::commit_pending_text()` and `EditSession::cancel_pending_text()`:
  the text lifecycle is now explicit-action driven and unit testable.
- `edit_proc` `WM_KILLFOCUS` no longer commits/destroys the EDIT: the draft and
  the box survive focus moving outside NameShot (and transitions to other
  NameShot controls); the message is forwarded to `DefSubclassProc` because the
  control now outlives the focus loss. Focus is never stolen back.
- Explicit commits only: `Enter` (in the box), toolbar/action commands
  (tool/colour/width, `Copy`, `Save`), `try_close`/`Cmd::Close`, overlay
  `WM_CLOSE`, tray *Quick save* and tray *Exit* (the last three so the dirty
  prompt and exported image account for the draft). `Esc` in the box cancels.
- Tests added (4): `external_focus_loss_preserves_the_pending_draft`,
  `app_owned_focus_transition_keeps_the_draft_until_explicit_commit`,
  `explicit_commit_places_trimmed_text_and_empty_box_closes_free`,
  `cancel_discards_the_draft_without_touching_history`.

### Harness changes (%TEMP%\opencode, not in repo)

- New shared `ns_guards.ps1`: `Set-StrictMode`, self-contained `PbG` P/Invoke
  (foreground, foreground-thread `hwndFocus`, window titles), `Assert-Fg`,
  `Assert-FgFocus`, `Invoke-KeyGuarded`. Every guard returns EXACTLY ONE
  `[bool]`; all logging goes through `Write-Host` so it can never pollute a
  captured result; `Invoke-KeyGuarded` blocks injection on non-boolean/null/
  `$false`/empty-guard results (fail closed).
- `ns_pb1.ps1`: strict mode; all four real-keystroke sites (Ctrl+Z, dialog N,
  typing Hola, Enter) routed through `Invoke-KeyGuarded`; typing/Enter require
  `hwndFocus == child EDIT`, Ctrl+Z requires `hwndFocus == overlay root`;
  artifacts uniquely timestamped (`qa_<stamp>_*.png`, guard log
  `pB_qa_<stamp>.log`); all output switched to `Write-Host`.
- New `ns_guards_test.ps1`: static failure-case coverage (fg mismatch, focus
  mismatch, zero focus, historical array return, null, empty guard list, later
  guard failing) - each must yield a single `[bool] $false` and must not run
  the action. Non-interactive: counters only, no keybd_event, no clipboard,
  no NameShot process.

### Gates (this phase)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Tests | `cargo test` | 0 | **66 passed, 0 failed** (58 lib incl. 4 new + 4 + 4) |
| Release build | `cargo build --release` | 0 | 0 warnings |
| Clippy | `cargo clippy` | 0 | 35 lib warnings = pre-existing baseline (0 in edit.rs) |
| Guard tests | `ns_guards_test.ps1` | 0 | **GUARD_TESTS=PASS** (26/26) |
| Harness parse | PowerShell AST, 3 scripts | 0 | 0 parse errors |
| Harness gate smoke | `ns_pb1.ps1` (no -ConfirmClipboard) | 2 | aborts before any side effect |
| Live QA | - | - | **NOT run; remains FAILED/unverified** |

**Status: text tool and full live QA remain UNVERIFIED.** This phase ran only
non-interactive automated tests. Live QA and packaging stay blocked pending the
user's review of this report and an explicit go-ahead.

## Phase H - Auto-close capture session: copy/save close, no discard prompts (2026-10-08)

Driver: user feature request — a capture session must end itself when the
user's intent is fulfilled: Copy closes only after the image really landed on
the clipboard, Save closes only after the bytes were written, and every
explicit close (Close button, `Esc`, `WM_CLOSE`, tray Exit) discards
immediately **without** the "Discard capture?" dialog. A failed copy, a
cancelled Save-As or a write error keeps the session open behind an in-app
error. No changes to hotkey, startup, settings, network or packaging.

### Interim context: authorized live run 20261008_163933 (harness workstream)

Between Phase G and this phase, the QA-harness workstream (fail-closed
guards, SMT proof) performed one authorized live run, stamp
`20261008_163933`, result **FAILED (23/10 as recorded by the run host)**.
Artifacts preserved in `%TEMP%\opencode\`: `pB_qa_20261008_163933.log`
(guard log), `pB_zdiag_20261008_163933.log` (window snapshots),
`qa_20261008_163933_pB_*.png` (step screenshots),
`pB_settings_backup_20261008_163933.ini`. Observable failure: `Esc` opened
the *Discard capture?* dialog (`zdiag: dialog=... above=['D']`) while the
foreground stayed the overlay, so the guarded `N` keystroke was blocked (fg
mismatch) and every later guarded key (Ctrl+Z, typing) was blocked — the run
aborted. This phase removes that dialog from the product entirely, which
also makes the harness's discard-dialog step obsolete.

### Design: one close policy (src/session.rs, new)

Pure module with no window types, exercised directly by tests:

- `Route`: `Copy{ok}`, `Save(SaveOutcome)` (`Saved`/`Cancelled`/`Failed`),
  `Close`, `TextCancel`, `Exit` — every way a session can end.
- `plan(route, session_open) -> Plan` (`KeepOpen`, `CancelTextDraft`,
  `CloseSession`, `Exit`): copy/save close only on success; `Close` always
  discards; `TextCancel` never touches the session; with no session open
  every route is a no-op except `Exit`, which still quits.
- `close_target(session_open, window_alive) -> CloseAction` (`Nothing`,
  `Destroy`, `ClearStale`): the idempotency table — a second finalize sees
  `(false, false)` and does nothing, so window and record are released
  exactly once.

Callers: `overlay::apply_route` / `editor::apply_route` plan and finalize;
`overlay::close_session` / `editor::close_session` run `close_target` against
the live record (`IsWindow` check, then `DestroyWindow` or stale-record
clear). No prompts, no `SetForegroundWindow`, no focus stealing;
`WM_DESTROY` still drops the record together with its child EDIT, fonts and
backbuffer.

### Production changes

- `src/session.rs` (new): policy + 9 state-machine tests.
- `src/lib.rs`: `pub mod session`.
- `src/overlay/mod.rs`: `close_session`/`apply_route`; `WM_CLOSE`,
  Selecting-phase `Esc`, editing-phase `Esc` and the toolbar Close command
  all route through the policy; `editing_dirty`/`confirm_discard` (the Yes/No
  prompt path) deleted; `edit::on_key` no longer takes the hwnd.
- `src/overlay/edit.rs`: Copy/Save close **after** clipboard/file success
  (the tray balloon replaces the status chip, which dies with the window);
  every failure/cancel outcome also routes through the policy (KeepOpen by
  design); `try_close`/`set_status` deleted; `Esc` in the focused text box
  cancels the draft only (`plan(TextCancel, open) == CancelTextDraft`), so
  the next `Esc` reaches the overlay and closes the session.
- `src/editor/mod.rs` (separate window, unreachable from the current flow):
  same contract — commit-before-copy/save in the toolbar command path,
  balloon + close on success, `close_session`/`apply_route`;
  `try_close`/`set_status` deleted.
- `src/app.rs`: tray **Exit** runs `apply_route(Exit)` then destroys the
  windows (no dirty check, no prompt); *Quick save* traces `Route::Save` on
  success **and** failure for whichever session's image it wrote (inline
  overlay frame or legacy editor frame); a fresh desktop capture closes
  nothing.
- `src/strings.rs`: `discard_title`/`discard_msg` removed;
  `win::dialog::confirm` remains as a utility but is no longer called.
- Annotation tools untouched: they never close the session.

### Routes after this phase

| Route | Result |
|---|---|
| Toolbar Copy / `Ctrl+C`, clipboard write ok | balloon + close |
| Copy failure (clipboard error) | in-app error, session open |
| Toolbar Save / `Ctrl+S`, file written | balloon + close |
| Save-As cancelled / write failed | session open (in-app error on failure) |
| Close button, `Esc` (no text box), `WM_CLOSE` | immediate discard, no dialog |
| `Esc` in the focused text box | draft cancelled, session open |
| Tray Exit | closes session (no prompt), then quits the app |
| Quick save of open session work | `Route::Save` traced; closes only on success |
| Annotation tools (pen ... solid) | never close |
| Second finalize (double click, late message) | no-op (`close_target` table) |

### Tests added (11; 66 -> 77 total)

- `src/session.rs` (9): copy/save/close/text-cancel/exit matrices, no
  failure route ever closes, `close_target` idempotency table, exactly-once
  finalize lifecycle, exhaustive full-route matrix (Exit-without-session is
  the documented exception).
- `src/overlay/edit.rs` (2): `committed_text_renders_into_the_exported_frame`
  (GDI glyphs land in `output_frame`, source bitmap immutable) and
  `solid_redaction_covers_committed_text_in_the_exported_frame`
  (redaction flattens over committed text through the real export path).

### Gates (this phase)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Format | `cargo fmt --check` | 1 | pre-existing: 198 hunks crate-wide; **0 in the new `session.rs`** and none of this phase's added lines flagged (flagged hunks in touched files are pre-existing lines/import order) |
| Tests | `cargo test` | 0 | **77 passed, 0 failed** (69 lib incl. 11 new + 4 + 4) |
| Check | `cargo check --all-targets` | 0 | 0 errors, 0 warnings |
| Clippy | `cargo clippy` | 0 | 35 = pre-existing baseline (0 from this phase's code) |
| Release build | `cargo build --release` | 0 | 0 warnings |
| Live QA | - | - | **NOT run; blocked pending user approval** |

### Docs and follow-ups (this phase)

- `README.md`: editing bullet rewritten (auto-close + tray balloon, immediate
  discard without confirmation, text-box `Esc` semantics) and the QA
  checklist item rewritten (the old "`Ctrl+C` then `Ctrl+S` pixel-identical"
  sequence is impossible once Copy closes the session).
- `docs/ARCHITECTURE.md`: no change needed (already describes save-and-close).
- Harness follow-up: `ns_pb1.ps1` still expects the discard dialog and an
  open overlay after copy — it must be updated before the next live run
  (harness lives outside this repository and was not touched this phase).

**Status: feature implemented and unit-verified; live QA not run.** Packaging
and any live/clipboard checks remain blocked pending explicit user approval.

## Visual refresh - modern-dark UI (2026-10-08)

Restyled every user-visible surface from one token set. Behaviour is out of
scope and unchanged: capture/annotation engine, in-overlay editing,
close-after-successful-Copy/Save, immediate no-confirm Close/Cancel,
redaction, settings semantics, a11y names, resource budgets. Design doc:
`docs/UI_REFRESH_SPEC.md` (surface inventory, tokens, per-surface plans,
icon set, constraint checklist, harness coupling).

### What changed

- **`src/theme.rs` (new, OS-free)**: single source of tokens - palette
  (`BG #101216`, `SURFACE #171A20`, `ELEVATED #20242C`, `CANVAS #0C0D10`,
  `BORDER/BORDER_INPUT`, `FG/FG_DIM/FG_DISABLED/ON_ACCENT`, one accent
  `#7AA7FF` + hover/down, `OK/WARN/ERROR` status-only, `SEL_OUTLINE`),
  control-state fills, type scale (title 22 semibold / ui 16 / small 13,
  Segoe UI), spacing 4/8/12/16, `RADIUS=4`, `BORDER_W=1`, icon metrics
  (18 px box, 2 px stroke, 3 px indicator), chrome geometry (kept values:
  `PAD=8 RAIL_W=60 RAIL_GAP=4 STRIP_H=46 CHIP_H=22`), `VEIL_KEEP_PCT=40`.
  Tests (6): COLORREF round-trip, zero alpha, WCAG >= 4.5 on essential
  pairs, surface-ladder ordering, token distinctness, type/spacing order.
- **`src/win/dark.rs`**: historic names re-export theme tokens; class brush
  is now `SURFACE` (Settings/About/hotkey/editor erase background); dark
  title bars unchanged.
- **Overlay** (`src/overlay/mod.rs`, `src/overlay/edit.rs`):
  - veil blends the frozen frame toward `BG` keeping 40 % of each pixel;
  - selection = 1 px `SEL_OUTLINE` outer + 2 px accent inner, corner
    handles 8x8 + edge bars (hit tolerance `HIT=7` unchanged);
  - new selection **size chip** (`ELEVATED` face, `W x H` in `FG_DIM`),
    flips above near the bottom edge; region invalidation 10 -> 32 px;
  - hint strip / rail / action strip sit on rounded `SURFACE` + 1 px
    `BORDER` panels;
  - tool + undo/redo buttons are **icon-only** (11 original GDI line icons
    in `src/editor/toolbar.rs`: Pen/Line/Arrow/Rect/Box/Text/Highlight/
    Mosaic/Solid/Undo/Redo; chip tooltips already covered every action -
    existing `tips_cover_every_button...` test still green);
  - Copy = accent primary face, Save = elevated secondary, Close = quiet;
    selected tool gets a 3 px indicator bar (shape cue, not colour alone);
  - status chip gains an **error state** (`ERROR` text + border); copy/save
    failures now report there instead of a modal box - session stays open,
    close policy untouched. Startup/tray/hotkey/settings errors keep the
    native message boxes.
- **Settings / hotkey / About** (`src/settings_ui.rs`):
  - group headers painted in `WM_PAINT` (`FONT_SMALL` on `FG_DIM`) with a
    1 px `BORDER_INPUT` divider: **Capture & Hotkey / Save & Format /
    Startup** (3 new EN strings; no control added or removed);
  - rows re-gridded on 4 px (client 484x368, was 484x300), same x/widths;
  - every control opts into `DarkMode_Explorer` (`dark_control`, defined
    before this phase but never called);
  - About fills `SURFACE`, title in the new semibold 22 px face, short
    accent divider under it (client 420x240, was 420x230);
  - ids, pending-settings model, a11y names, hotkey capture logic: not
    touched. Layout constants + `WM_PAINT` grouped via shared consts.
- **`src/app.rs`**: `make_font(px, weight)` + new `font_title`; sizes come
  from `theme::FONT_*_PX` (values identical to the old literals 16/13).
- **Legacy editor** (`src/editor/mod.rs`): canvas border moved to the
  selection token for coherence; rest already routed through `dark::`.
- No magic RGB literals remain outside `theme.rs` (source-wide grep).

### Tests (77 -> 94: 69 -> 86 lib, integration unchanged)

theme 6 (tokens/WCAG/ladder/distinctness); toolbar 6 (icon ASCII render via
DIB + distinctness, emphasis mapping, layout bounds/icon-tools/text-actions/
Close-pinned/narrow); overlay mod 2 (veil math, chrome geometry); overlay
edit 2 (rail/strip edge-aware placement, status-error lifecycle); settings 3
(4 px grid, group rows vs headers/dividers, header strings).

### Gates (this phase)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Check | `cargo check --all-targets` | 0 | 0 errors, 0 warnings |
| Tests | `cargo test` | 0 | **94 passed, 0 failed** |
| Release build | `cargo build --release` | 0 | 0 warnings |
| Format | `cargo fmt --check` | 1 | pre-existing, repo-wide (crate never rustfmt'd; first diff `annotate.rs`, untouched) - unchanged condition, no mass reformat performed |
| Clippy | `cargo clippy --all-targets` | 0 | **40 warning lines <= 42 baseline** (0 from this phase's code after scoped `assertions_on_constants` allows on the token/layout tests) |
| Live visual QA | - | - | **NOT run; exact steps below await approval** |

### Harness coupling (follow-up task, harness not touched)

`%TEMP%\opencode\ns_pb1.ps1` (and `ns_fx3.ps1`) hardcode the pre-refresh
palette; update before the next live run:

| Constant | Old | New | Why |
|---|---|---|---|
| `$CVb,$CVg,$CVr` (canvas probe, BGRA bytes) | `17,18,20` (`#141211`) | `16,13,12` (`#0C0D10`) | canvas fill token |
| `$BBb,$BBg,$BBr` (rail/strip fill probe) | `42,45,51` (`#332D2A`) | `32,26,23` (`#171A20`) | rail/strip panels and resting buttons are now `SURFACE` |
| `$STRIP` X coords | measured on text-width undo/redo | re-measure | undo/redo are 40 px icons, strip ideal width shrank a few px (buttons are wider than the shift, clicks still land, centers drift) |
| `$RAIL_X` / `$RAIL` Ys | unchanged | unchanged | rail geometry (60 px wide, 42 px pitch) identical; tools click by coordinate, not label |

Canvas probes that compare counts against their own baseline still work
(`CountColor` both sides).

### Evidence steps for approval (opens windows; no capture, no clipboard)

1. Run `target\release\nameshot.exe` (freshly built) -> tray icon appears.
2. Tray > **Capture** -> drag a selection -> screenshot: veil, selection
   chrome, size chip, hint strip.
3. `Enter` -> screenshot: icon rail + panel, action strip (Copy accent,
   icon undo/redo), status chip.
4. Tray > **Settings** -> screenshot: group headers + dividers, dark
   controls; **Change...** -> hotkey window; tray > **About** -> screenshot:
   title + accent divider.
5. `Esc` -> `Esc`; tray > **Exit**.

Packaging NOT run this phase. `dist\NameShot-0.1.0-win-x64-20261008_201524.zip`
(797,141 bytes, SHA256 `42CABBF0...3996`) untouched.

**Status: implemented and unit-verified; live visual QA, harness constant
update and re-packaging remain blocked pending explicit user approval.**

## Selection on mouse release - no-Enter editing entry (2026-10-08)

User directive: selection-on-mouse-release as a separate focused task; the
refreshed visual design must not change; harness updates limited to the new
colors/layout and the no-Enter flow; non-interactive checks only, then stop
for review.

### Change (app)

- `src/overlay/mod.rs`
  - New `ReleaseAction { Commit, Revert, Stay }` with
    `fn release_action(drag, sel)` as the entire policy: an `ODrag::New`
    that moved and is at least `theme::SPACE_XS` (4) px in both dimensions
    commits (`Commit` -> `confirm` -> `Phase::Editing`); no movement or a
    band below 4x4 restores the previous selection (`Revert`, click noise);
    `Move`/`Resize` releases only end the drag (`Stay`), so adjustments keep
    the selecting phase.
  - `on_up` executes that policy and finally uses its `hwnd` argument:
    `Commit` calls `confirm(app, hwnd)` directly on mouse-up; `Revert`
    invalidates the union of the current band and the restored selection
    (chrome room included by `invalidate_region`).
  - The moved flag is sticky (`band_moved`): once the band left its starting
    rectangle, returning to it still counts as moved, so a full-screen band
    commits on release instead of being mistaken for a click.
  - Enter (`on_key`) and double-click still call `confirm` - the keyboard
    path is unchanged; `confirm` early-returns when already editing, so a
    late mouse-up after Enter is a no-op; `WM_CAPTURECHANGED` still drops a
    drag without committing; `capture_empty_msg` still guards tiny Enter
    selections.
  - Module doc comment rewritten for the new flow. No palette, chrome,
    settings, About or geometry code was touched.
- `src/strings.rs`: `capture_hint` -> `"Drag to select - release to edit"`.
  `capture_hint_keys` keeps `Enter: use` (keyboard path still valid).
- Tests added (lib 86 -> 88): `mouse_release_commits_a_real_rubber_band_only`
  (commit / revert / 4 px threshold on both sides / move + resize stay) and
  `band_movement_is_sticky_across_a_return_to_the_start`.

### Harness `%TEMP%\opencode\ns_pb1.ps1` (outside repo, updated as approved)

- Backup of the previous version: `ns_pb1_before_release_20261008_212919.ps1`.
- Palette constants (BGRA bytes): `$CV = 16,13,12` (`#0C0D10`), `$BB =
  32,26,23` (`#171A20`) - the probes were still on the pre-refresh palette
  (`17,18,20` / `42,45,51`), which made the canvas/surface counts trivial.
- Geometry: strip probe -> `(510, 980, 670, 46)` (the actual
  `EditSession.strip` panel); `$STRIP` -> button centers `Undo=538,
  Redo=582, Copy=1026, Save=1082, Close=1142`. Values come from a temporary
  layout-dump test of `EditSession::new(crop (200,250)-(1500,950), client
  1920x1200)` (printed rects, then the test was removed). `$RAIL_X`/`$RAIL`
  and `$STRIP_Y` unchanged - rail pitch and strip center line did not move.
- Flow: `Enter-Edit` (VK_RETURN messages) removed; `Drag-Region` = down +
  moves without mouse-up (captures the selecting state), `Release-Region` =
  mouse-up + 300 ms, `Select-Region` = both. S1 now captures
  `pB_02_selected` mid-drag, releases, asserts same-hwnd, then proves the
  editing phase from `pB_03_editing` rail/strip SURFACE probes ("editing
  entered on mouse release without Enter"). S2/S3/S5 assert "after mouse
  release". No selection code path posts Enter anywhere (S5's `Enter` stays -
  it is the text-tool commit). Script parses clean
  (`[Language.Parser]::ParseFile` -> 0 errors); UTF-8 no-BOM + LF line
  endings restored after editing.
- Untouched: `ns_checks.ps1`, `ns_checks_test.ps1`, `ns_guards.ps1` (shared
  libraries), `ns_fx3.ps1` (legacy), older `ns_pb1_before_*` backups.

### Gates (non-interactive, this task)

| Check | Result |
|---|---|
| `cargo check --all-targets` | exit 0, 0 warnings/errors |
| `cargo test` | 96 passed (88 lib + 4 int + 4 doc), 0 failed |
| `cargo clippy --all-targets` | exit 0; 41 warning headers, diagnostic set identical before/after the fmt cleanup; none in the new code (`src/annotate.rs:84` derivable-impl is pre-existing, file untouched) |
| `cargo fmt --check` | exit 1 as the recorded baseline: pre-existing repo-wide diffs (27 files); no hunk contains any line added or edited by this task |
| `cargo build --release` | exit 0 |
| harness parse + encoding | `ParseFile` 0 errors; no BOM, LF only |

Docs updated: README features bullet + QA checklist (release-to-edit flow),
spec section 6 (harness note - constants and flow now aligned).

Live QA / screenshots: NOT run (blocked pending approval).
Re-packaging: NOT run; `dist\NameShot-0.1.0-win-x64-20261008_201524.zip`
(797,141 bytes, SHA256 `42CABBF0...3996`) untouched.

**Status: implemented and unit-verified; stopped for review as directed - no
live QA, no packaging.**

## Full live QA + release packaging (2026-10-08 21:55)

Approved run of the updated `%TEMP%\opencode\ns_pb1.ps1` (selection-on-
mouse-release flow, refreshed palette/geometry constants) against
`target\release\nameshot.exe`, executed exactly once with
`-ConfirmClipboard`. The operator force-killed nothing; a guard failure
would have made key injection impossible (fail closed) - none failed.

- Run stamp `20261008_215513`: `PHASE_H_LIVE=PASS`, **PASSED=72,
  FAILED=0**, exit 0.
- Release-flow checks: S1/S2/S3/S5 all entered editing on mouse release
  without Enter (`rail SURFACE=17865`, `strip SURFACE=14497` on S1;
  `SAME overlay hwnd after mouse release` asserted in S2/S3/S5); no
  `NameShot.Editor` window and no #32770 dialog at any point.
- Session contracts: S1 dirty-Esc immediate close; S2 Copy -> valid CF_DIB
  1301x701 + auto-close; S3 Save -> Cancel keeps the session open, zero
  files written; S4 Save -> exact %TEMP% path match, 50,638-byte file,
  auto-close; S5 guarded typing `hola` + red-glyph asserts + final Esc.
- Safety: settings.ini backed up and restored (`ini_restored=True`), QA
  save folder removed, no `NameShot_*` leak in Pictures; artifacts:
  guard log `pB_qa_20261008_215513.log`, zdiag
  `pB_zdiag_20261008_215513.log`, screenshots `qa_20261008_215513_*`
  (20 states), backup `pB_settings_backup_20261008_215513.ini`.
- Cleanup: app closed via the harness's polite WM_CLOSE path
  (`cleanup alive=False`); process count 0 afterwards.

### Package

- New: `dist\NameShot-0.1.0-win-x64-20261008_215657.zip` (804,618 bytes)
  containing `nameshot.exe` (2,139,597 B), `README.md` (12,907 B),
  `docs\ARCHITECTURE.md` (10,360 B), `docs\BUILD_LOG.md` (93,423 B).
- Packaged exe SHA-256
  `4C7845D69BA25D2D6B85C76534B5E05C1C102045EDF366C2BC65927E908E03AE`
  == `target\release\nameshot.exe` (the tested build), verified by
  extracting `nameshot.exe` from the archive and hashing it.
- Previous package preserved byte-for-byte:
  `dist\NameShot-0.1.0-win-x64-20261008_201524.zip` (797,141 bytes,
  SHA-256 `42CABBF0...3996`).

**Status: live QA PASS (72/0) and release package created; deliverable
ready.**

---

## Small-selection chrome layout + project icon audit (2026-10-08, Phase 0 diagnosis)

Scope: two user-reported defects in the delivered build — (a) a 330x207
selection shows the tool rail crossing a long horizontal action strip that
runs to the screen edge; (b) the app still ships what our own docs call a
generated/placeholder icon instead of the real project asset. Diagnosis
only; constraints for the fix: no live-QA run, no key injection, no
packaging/new ZIP, keep the 96-test suite green, preserve both `dist` zips
and all QA artifacts.

### A. Layout root cause — `src/overlay/edit.rs::rebuild_bar` (lines 209-321)

- **Rail** (226-250): button height `th = clamp((client_h - 2*PAD - gaps)/9,
  20, 38)` — on any client taller than ~440 px this yields `th=38`,
  `rail_h=374`. `rail_y = canvas.top.clamp(4, client_h - rail_h - 4)`: for a
  selection shorter than the rail (207 px vs 374 px) the rail overhangs the
  selection by up to +167 px. `rail_x` prefers left of the canvas, else
  right, else clamps into view (possibly onto the canvas). Nothing
  coordinates the rail with the strip or the chip.
- **Strip** (286-312): `x0 = selection_center - ideal/2` clamped to
  `[PAD, client_w - ideal - PAD]`; `ideal ~= 670 px` is wider than a 330 px
  selection, so the strip spans ~170 px past each selection edge and, when
  the selection is off-center, the clamp pins it flush to a screen edge.
  `strip_y = canvas.bottom + CHIP_GAP(30)` when it fits, else above at
  `canvas.top - CHIP_GAP - STRIP_H`, else pinned to the bottom edge.
- **Status chip** (313-319): docks at `strip_y -/+ 4`, x = strip.left,
  width `min(340, ...)`. Its x band can also reach across the rail column.
- **No pairwise rejection**: `bar.rect = union(rail, strip)` (320) and
  `compose` paints the rail panel first, strip panel second
  (`edit.rs:1006-1007`), so a crossing pair renders as two panels painted
  through each other — exactly the reported screenshot.

Worked example reproducing the report (client 1920x1200, selection
`(1570, 910, 330, 207)`):

- rail: `th=38`, `rail_h=374`, `rail_x=1502` (left fits), `rail_y =
  clamp(910, 4, 822) = 822` → rail `(1502,822)-(1562,1196)` — overhangs the
  selection both ends; its y band contains the strip band.
- strip: `x0 = clamp(1735-335, 8, 1920-670-8) = 1242` → flush to the right
  edge; selection bottom 1117 makes `1117+30+46 = 1193 > 1192`, so the
  strip flips above at `910-76 = 834` → strip `(1234,834)-(1920,880)`.
- intersection `rail ∩ strip = (1502..1562) x (834..880)` = 60x46 px of
  panels painted through each other; the chip `(1234,884)-(1574,906)` also
  crosses the rail column. Rail buttons stay visible above and below the
  strip — matches the report.
- Same mechanism covers the right-side rail (the wide strip reaches the
  right rail column) and the strip-below case (rail bottom hangs into the
  below-selection band whenever `sel_h < rail_h - CHIP_GAP - STRIP_H
  ~= 298`).

Existing contract to keep (`edit.rs:1597-1644`
`rail_and_strip_are_icon_first_and_edge_aware`): rail prefers the free left
side and flips right when flush left; strip prefers below and flips above
when flush bottom; chip docks exactly `strip.top-4` / `strip.bottom+4`; all
rects inside the client.

Planned fix (Phase 1): a pure, unit-testable layout function
(measure → generate candidates → reject out-of-bounds/overlapping → rank:
not covering the selection, then minimum movement, then the larger
Expanded metrics) returning `Expanded | Compact | EdgeFallback` plus
rail/strip/chip rects and rail button height; `rebuild_bar` keeps building
buttons from those rects. Hard invariants: all rects inside the client,
rail/strip/chip pairwise disjoint; recomputed only where `rebuild_bar`
already runs (state/selection/client change — no polling).

### B. Icon audit

- **Intended source**: `assets/icon.png` (2,350,739 B, 2508x2508, 8-bit
  RGBA, transparent margins; navy rounded square, blue crop brackets, white
  pen with blue stroke). Unambiguous — it is the only artwork in `assets/`.
- **`assets/icon.ico`** (49,611 B): type=1, four 32-bpp frames
  16/32/48/256, generated 2026-07-10 by `assets/make_placeholder_icon.ps1`
  whose header labels itself "TEMPORARY placeholder generator"; README
  (lines 113-125, 170, 192-193) documents the whole icon chain as
  placeholder. Rendered frames show the same art as `icon.png` (faithful
  derivation, but still the stopgap artifact the task retires).
- **Executable**: `build.rs` embeds `1 ICON "assets/icon.ico"` via
  windres; the exe carries exactly 1 icon group (ExtractIconEx count = 1)
  and the extracted large icon renders the intended logo (evidence
  `%TEMP%\opencode\exe_icon_large.png`). The file icon is content-correct
  today; the rest of the chain is not.
- **Tray**: `src/win/tray.rs::draw_icon` (line 176) builds the tray icon
  procedurally at runtime (module doc: "programmatic icon (no resources
  needed)") — a drawn approximation (rounded square + corner brackets, no
  pen): the generated icon users actually see every session.
- **Window classes**: `src/app.rs:397-410 register_class` registers every
  class (main, overlay, settings, about, hotkey) with
  `hIcon: HICON::default()` / `hIconSm: HICON::default()` (NULL), so title
  bars, Alt-Tab and taskbar buttons of Settings/About/hotkey rely on shell
  fallback instead of carrying the app icon.
- **About surface**: `settings_ui::paint_about` (880-953) draws title +
  accent divider + text only — no icon image is displayed, so nothing to
  re-wire there without redesigning the window (decision: leave About
  content unchanged; its title-bar icon comes from the class fix).

Planned fix (Phase 2): regenerate `assets/icon.ico` in place as the final
multi-resolution .ico from `assets/icon.png` with a documented,
reproducible command (new `assets/make_icon.ps1`; source png and legacy
placeholder script preserved), keep `build.rs` pointing at
`assets/icon.ico`, load the embedded resource for the tray icon
(`LoadImageW` on this module sized `SM_CXSMICON`, `DestroyIcon` on drop —
`draw_icon()` kept only as resource-missing fallback) and set
`hIcon`/`hIconSm` in `register_class` from the same resource. Verification:
icon-group count, frame sizes, pixel comparison against `icon.png`, hashes
recorded in the implementation entry below.

## Small-selection chrome layout + project icon — implementation (2026-10-08, Phases 1-4)

Implements both planned fixes above. Deliberately stops before live QA and
packaging: `ns_pb1.ps1` was never run for this task, no key injection, no
clipboard/settings changes, no saves outside the repo, no packaging, no new
ZIP; both `dist` zips, all QA artifacts and `assets/icon.png` untouched.

### Phase 1 — adaptive chrome layout (pure, unit-testable)

- New `src/overlay/layout.rs` (declared in `src/overlay/mod.rs`): pure
  `plan(&Measures) -> Layout` — **measure → generate candidates → reject
  (client bounds, pairwise overlap) → rank** (never covers the canvas,
  then minimum movement, then profile preference, then larger button
  height) → mode `Expanded | Compact | EdgeFallback`.
  - Candidate profiles: *Expanded* = today's metrics (`th_max`,
    `gap = rail_gap`); *Compact* sweeps `th` down `th_max → th_min →
    th_floor` and shrinks the rail gap to 0; *EdgeFallback* additionally
    pins the rail x and strip band/x to the screen edges. Rail sides
    left/right/clamped; strip bands below/above/pinned; strip x centered
    or shifted clear of the rail column; chip docked `strip.top-4` /
    `strip.bottom+4` with side flips (today's rule) plus a rail-clearing
    x shift.
  - Hard invariants enforced by construction and by 9 unit tests: every
    rect inside the client, rail/strip/chip pairwise disjoint (the
    reported crossing becomes unconstructible), chip always docked ±4
    (empty chip only in the degenerate-client `emergency()` branch).
  - Mode rule: `EdgeFallback` when pinned/covering, `Expanded` when rank-0
    at full metrics, otherwise `Compact`.
- `src/overlay/edit.rs::rebuild_bar` was rewritten to consume
  `super::layout::plan` and only stack buttons into the chosen rects
  (`bar.rect` stays `union(rail, strip)`); paint path, invalidation and
  state handling untouched. The pre-existing contract test
  `rail_and_strip_are_icon_first_and_edge_aware` (icon-first, edge-aware,
  chip docking ±4) passes unchanged.

### Phase 2 — real project icon end-to-end

- `assets/make_icon.ps1` (new, documented final generator) ran once:
  `assets/icon.png` → `assets/icon.ico` = **69,509 B, 7 frames
  16/24/32/48/64/128/256, 32 bpp RGBA, PNG-compressed, square**, SHA256
  `F420706A91032CA06FB52DF7F9F5144C22B6A264488E6CCF2AC3C29D7ED68FED`
  (previous placeholder .ico: 49,611 B, 4 frames, generated 2026-10-07 by
  the self-labeled `assets/make_placeholder_icon.ps1`, SHA256
  `1B68ACDCB7F2725732A38FB70F9880705538C9412B88AFF82F1BC644703BD093`,
  replaced in place; the legacy script is kept, superseded, untouched).
  Source `assets/icon.png` SHA256
  `C0609FD2EFF70A5AF8655D33A1D8227B2AD9E89F6A9A4F6E0303BDF4361D2125`
  (read-only throughout).
- Rust wiring (`build.rs` unchanged — `cargo:rerun-if-changed` picked up
  the new .ico):
  - `src/win/mod.rs`: `app_icon(small)` — `OnceLock<isize>` cache,
    `LoadImageW` on this module for resource id 1 (`icon_resource()`,
    `MAKEINTRESOURCE(1)`), sized `SM_CXICON`/`SM_CXSMICON`, falls back to
    `HICON::default()` if the resource is missing; `owned_app_icon(size)`
    returns an owned handle for the tray.
  - `src/app.rs::register_class`: `hIcon: win::app_icon(false)`,
    `hIconSm: win::app_icon(true)` on every class (was `HICON::default()`).
  - `src/win/tray.rs::TrayIcon::add`: uses `owned_app_icon(size)`;
    `draw_icon()` retained only as resource-missing fallback (module doc
    updated accordingly).
- Verification of `target\release\nameshot.exe` (fresh build; scripts
  `%TEMP%\opencode\verify_exe_icon.ps1`, rendered PNGs in the same dir):
  - exe SHA256
    `92BFA4F53E80FA735AD59EB9EB30B74D481FD4F8730B00CF9DAC48C0F72AD866`.
  - Resource table: `RT_ICON` ids `1..7`, `RT_GROUP_ICON` id `1`, and
    `FindResource(..., name=1, RT_ICON)` succeeds — exactly what
    `app_icon()`/`owned_app_icon()` request.
  - All **7/7 RT_ICON frame payloads inside the exe are byte-identical**
    to the 7 frames parsed from `assets/icon.ico` (order-independent
    SHA256 set comparison) — windres embeds the PNG frames as-is.
  - Large and small renders of the exe icon vs the .ico icon (same API
    path both sides) are hash-identical: large
    `6102AA70FD61FB7CB7B882775081EAA9EF78888A2C7EF4A01B0ABA2BAE410C0F`,
    small
    `69CC06748D634B9A3573398CA1CCE1690121CB43D59793BA26921ADF3C85E5C7`;
    visual check of both = the intended logo (navy rounded square, blue
    crop brackets, white pen).
  - About window content left unchanged per diagnosis (its title-bar icon
    now comes from the class fix).

### Phase 3 — gates (exact results)

- `cargo check --all-targets`: exit 0, **0 errors, 0 warnings**.
- `cargo test`: exit 0, **107 passed / 0 failed** = 97 unit (88 baseline +
  9 new `layout` tests) + 10 integration (4 baseline `encode_roundtrip`,
  4 baseline `redaction`, 2 new `icon_asset`); baseline was 96
  (88 + 4 + 4).
- `cargo clippy --all-targets`: exit 0; 38 warnings, **all pre-existing**
  (none in `layout.rs`, `icon_asset.rs`, `win/mod.rs`, `tray.rs::add` or
  `rebuild_bar`). The two `manual_dangling_ptr` hits introduced while
  wiring the icon were fixed with `icon_resource()` plus a targeted
  `#[allow]`: clippy's suggested `std::ptr::dangling::<u16>()` is
  align-of(u16) = 2, which would silently request resource id 2 instead of
  1 — the explicit `1usize as *const u16` is the correct MAKEINTRESOURCE.
- `cargo fmt --check`: exit 1 = **exactly the 27 pre-existing dirty files**
  (repo-wide baseline); `layout.rs`, `icon_asset.rs` and the touched
  regions of `win/mod.rs`/`edit.rs` are rustfmt-clean (no new diffs).
- `cargo build --release` (windres embedding the new .ico): exit 0.

### Phase 4 — documentation

- This entry (diagnosis above, implementation here).
- `docs/UI_REFRESH_SPEC.md`: new "Editor chrome placement (adaptive)"
  subsection under §3; rail/strip placement bullets and the tray icon
  bullet now point at the planner/resource instead of the old fixed
  placement and the token-drawn icon.
- `README.md`: icon section rewritten as the final chain (`icon.png` →
  `make_icon.ps1` → `icon.ico` → `build.rs` → window/tray resources,
  `tests/icon_asset.rs` locks the structure), tray-icon "temporary
  placeholder" limitation removed, project-layout lines and the test
  count (107) updated; legacy `make_placeholder_icon.ps1` documented as
  superseded, kept for reference.

## Taskbar-safe chrome (rcWork) + tray re-add recovery (2026-10-09)

### Problem (from the reviewed baseline visual check)

- Editor chrome (rail/strip/status chip) was planned against the full
  client, i.e. the whole monitor rect, so a low selection let the rail
  spill onto the taskbar band: baseline scenario A rail bottom ≈ y1193,
  scenario D ≈ y1190 on the 1920x1200 screen (work area bottom y1160).
- `tray::readd()` only issued `NIM_MODIFY` while `added == true`, so
  after an Explorer restart (`TaskbarCreated`) the icon never returned
  for the rest of the session: `NIM_MODIFY` cannot resurrect an entry
  the restarted shell no longer knows, and `added` never flipped back.

### Fix

- `overlay/layout.rs`: `Measures` gained `work: RECT` (the selected
  monitor's `rcWork`, clipped to the client); `bounds_of()`/`in_bounds()`
  now bound every rail/strip/chip candidate — `plan()`, `find_chip()`,
  `emergency()` — to it. An empty or disjoint work rect falls back to
  the full client, so with a full-client work area every candidate is
  identical to the pre-fix behavior (existing layout tests unchanged).
- `overlay/mod.rs`: new pure `work_to_client()` translates the screen
  work area into overlay-client space; negative monitor origins and the
  excluded taskbar band survive unchanged (unit-tested for a dual-head
  setup with virtual origin on the right monitor). The commit path
  computes `work = work_to_client(os::work_area_for(&sel_screen), &vs)`
  and passes it to `EditSession::new`.
- `overlay/edit.rs`: `EditSession` stores `work` and feeds it to the
  planner; end-to-end test asserts rail/strip/chip stay above y1160 for
  the two low-selection canvases.
- `win/tray.rs`: `readd()` is `NIM_ADD`-first with the complete payload
  (callback, icon, tooltip), falls back to `NIM_MODIFY` only when the
  add fails (entry already present), and records the real outcome via
  the pure `recovery_state(add_ok, modify_ok)` so a total failure flips
  `added` to false and the next broadcast retries `NIM_ADD`. Payload
  building moved into a single `nid()` used by every shell operation so
  ADD/MODIFY cannot drift. No Explorer restart/kill at any point.

### Tests (+7, all driving the real production logic)

- `layout`: `bottom_taskbar_work_area_keeps_all_chrome_above_it`
  (scenarios A, D, plus a selection hanging into the taskbar band),
  `work_area_offset_to_a_secondary_monitor_stays_confined` (work rect at
  x 1920..3840 on a 3840x1200 virtual screen),
  `empty_or_disjoint_work_rect_degrades_to_the_client`.
- `edit`: `chrome_never_enters_the_taskbar_band_of_the_work_area` —
  `EditSession::new` → `rebuild_bar` → `layout::plan` for the low
  canvases, asserting every non-empty chrome rect ends ≤ 1160.
- `overlay`: `work_area_maps_from_screen_to_client_with_negative_monitor_origins`
  (both monitors, 1160-height preservation).
- `tray`: `recovery_state_is_add_first_with_modify_fallback` (matrix),
  `nid_payload_is_complete_for_add_and_modify` (distinct fake
  HWND/HICON via transmute, all flags set, tooltip ==
  `s().tray_tooltip`; `Shell_NotifyIconW` never called from tests,
  `mem::forget` prevents `Drop` from touching the shell).

### Gates (all green)

- `cargo test`: **114 passed / 0 failed** = 104 unit + 10 integration
  (baseline 107 = 97 + 10; +7 from this entry).
- `cargo clippy --all-targets`: exit 0; no warning points at a changed
  line (every hit is pre-existing code, line numbers shifted only).
- `cargo fmt --check`: exit 1 = **exactly the 228 pre-existing hunks**
  repo-wide; each touched file matches its baseline count
  (`overlay/layout.rs` 0, `win/tray.rs` 4, `overlay/edit.rs` 36,
  `overlay/mod.rs` 24) — the 11 initially non-conformant new hunks were
  hand-formatted to rustfmt style instead of running `cargo fmt`.
- `cargo build --release`: exit 0; tested exe SHA-256
  `6A9B25E539ED207DB5763EF2C352875BFFCACDCF9B785EF37AE137DD5A32EF5C`.

### Live visual check (`qa_20261009_175640_NameShot_visual`)

- Scenario D (selection bottom y1150): rail bottom ≈ y1155, strip
  ≈ y958, chip ≈ y911 — all above the taskbar (y1160); the strip
  correctly flips above the selection because `below` no longer fits
  inside the work area.
- Scenario A: strip y835-880, chip "331 x 208", rail ends ≈ y1150
  (baseline crossed the taskbar at ≈ y1193).
- Tray: overflow-flyout cell present, tooltip "NameShot - Print Screen
  to capture", `zoom_nameshot.png` matches `ref_icon_32.png`.
- Exit graceful (`exited=True`), explorer PID 5132 unchanged; no
  clipboard harness, no settings writes.


## M0 cross-platform seam refactor (2026-10-09)

Approved scope: M0 only (seam + cfg gates; no Linux/macOS runtime code, no packaging).
Goal: enforce the `platform` seam with zero Windows behaviour change, and make the core
`cargo check` clean for x86_64-unknown-linux-gnu and aarch64-apple-darwin.

### What changed

- New `src/platform/`: `mod.rs` cfg-dispatches to `windows.rs` / `linux.rs` / `macos.rs`
  (catch-all -> linux.rs). Windows re-exports `crate::text::draw_text`; Linux/macOS stub
  it with `unimplemented!` ("portable text rasterization arrives in M1"). `flatten.rs`
  now calls `crate::platform::text::draw_text` (same function on Windows).
- `src/lib.rs`: `#[cfg(windows)]` on app, capture, editor, encode, os, overlay,
  settings_ui, text, win; non-Windows `overlay` is `#[path = "overlay/layout.rs"]` so the
  OS-free planner keeps compiling; `run()` cfg-split (Windows calls `app::run()`, other
  targets print a port-not-ready line and return 1).
- `src/error.rs`: `Error::Win32(windows::core::Error)` variant, its `From` impl, and its
  two match arms gated `#[cfg(windows)]`; `with_context` gets
  `#[cfg_attr(not(windows), allow(unused_variables))]`.
- `src/overlay/layout.rs`: pure core `Rect { left, top, right, bottom }` (Debug, Clone,
  Copy, Default, PartialEq, Eq) replaces the `windows` `RECT` import; the file now has
  zero OS imports (fmt count stays 0).
- `src/overlay/edit.rs`: `From<Rect> for RECT` + `From<RECT> for Rect` at the boundary;
  `Measures` inputs `.into()`, `self.rail` / `self.strip` / `self.status_rect` outputs
  `.into()`.
- `Cargo.toml`: `[dependencies]` -> `[target.'cfg(windows)'.dependencies]`.
- `src/hotkey.rs`: doc link `[`crate::win::hotkey`]` -> plain code span (module is
  cfg(windows)).
- `build.rs` was already target-gated; no change.

### Gates (all green)

- `cargo check --target x86_64-unknown-linux-gnu`: exit 0, zero warnings.
- `cargo check --target aarch64-apple-darwin`: exit 0, zero warnings.
  (rustup targets added; this machine has no WSL, so `cargo check` is the only available
  non-Windows verification.)
- `cargo test`: **114 passed / 0 failed** (unchanged).
- `cargo clippy --all-targets`: exit 0; 33 lib / 39 total warnings = baseline; no
  warning points at a changed file (error.rs, flatten.rs, layout.rs, edit.rs, lib.rs,
  platform/*: zero hits).
- `cargo fmt --check`: exactly **228** = 228 baseline; every touched file at baseline
  (layout.rs 0, edit.rs 36, error.rs 2, flatten.rs 11); the 2 new edit.rs hunks from the
  `From` impls were hand-formatted to rustfmt style.
- `cargo build --release`: exit 0. SHA-256
  `676AE653DBB2430E12892787FED8DA2648529758530A4866CE708595FDCD43E3`
  (previous packaged exe `6A9B25E5...EF5C`).

### Exe diff-explanation (byte-comparable OR diff-explained)

Old exe extracted from `dist/NameShot-0.1.0-win-x64-20261009_181155.zip` (hash verified
against the package manifest) and compared section-by-section and symbol-by-symbol
(Python COFF parse):

- All 11 PE sections have byte-identical sizes (`.text` 1026560, etc.); total +21 bytes.
- 4654 functions in both; 4645 common by name; 4644 with identical code size. The single
  measured mismatch (`register_frame_ctor`, CRT code, 144 vs 160) is a symbol-span
  artifact (its neighbors moved; the CRT object file is precompiled and untouched).
- The 9 only-old / only-new pairs are the same `overlay::edit` methods with their
  impl-block disambiguator shifted by the two new `From` impls, plus `.llvm.<hash>`
  local-symbol renames; `rebuild_bar` 3664 -> 3648 bytes (span of the new inlined
  boundary code).
- Remaining .text churn is relocation: functions keep their sizes but move, so every
  RIP-relative displacement differs. No instruction volume changed anywhere.

### Windows QA smoke (`qa_20261009_185813_M0_smoke`)

- The resident dist instance was exited gracefully via its own tray menu first, the new
  build was exercised, then the dist instance was relaunched so the environment was left
  as found.
- Scenario D: chrome matches the baseline shot at pixel level (chip `#20242C` at the
  identical crop coordinates; strip y914-960; rail bottom approx y1153-1157) - all above
  the taskbar band (y1160).
- Scenario A: chip "331 x 208", strip y835-880, rail ends approx y1150 - above y1160.
- Tray: overflow flyout opened (10 icons), icon hover OK (`hover_nameshot.png`).
- Exit: menu Exit -> `exited=True`, explorer PID 5132 unchanged, no error events.
- Two earlier process deaths were environmental, not app faults: one was a tool-timeout
  tree-kill of a redirected child; one had no WER/panic trace during a period of active
  user mouse input and the immediate re-run completed every scenario.

### Not done (needs separate approval)

M1 Linux X11 adapter code, WSL installation, any non-Windows build/package, any README
support wording.

## M1 - Linux X11 adapters: implementation + first gate pass (2026-10-10)

User-scoped: X11 only (no Wayland/macOS/packaging); Windows build, resident PID 9420,
and all existing `dist` ZIPs preserved (verified untouched: `dist\nameshot.exe` still
`6A9B25E5...EF5C`); no runtime claims without an X11 session; stop for review after the
gates.

### M0 record note (fmt exit code)

The M0 `cargo fmt --check` runs recorded the exact 228-hunk count, but their exit codes
were not captured for M0. The M1 run below records command, exit code, and hunk count
together.

### Implemented (source changes)

- `src/platform/x11/{mod,display,capture,encode,hotkey,text,clipboard,dialog,windowing}.rs`
  - the nine X11 adapter modules, `cfg(target_os = "linux")` via `platform/linux.rs`.
- `src/platform/x11_logic.rs` - pure protocol/codec helpers (ZPixmap BGRA conversion,
  keysym/modifier tables, Xft DPI parsing, font-path picking, zenity arg building,
  grab-conflict mapping) + unit tests that run in the regular Windows suite.
  Compiled `#[cfg(any(test, not(windows)))]` - rationale in the hash section below.
- `src/platform/linux.rs` + `src/platform/mod.rs` - Linux dispatch arms, `x11_logic`
  module, docs.
- `src/lib.rs` - `#[cfg(not(windows))]` run() message (M1 status text).
- `src/config.rs` - XDG split: `#[cfg(windows)]` config_dir (body unchanged), new
  `#[cfg(not(windows))]` variant, pure `config_dir_from` (gated
  `cfg(any(test, not(windows)))`) + `xdg_config_dir_resolution` tests (4 assertions).
- `src/output.rs` - `build_bmp` (BMP file = 14-byte header + `build_dib` body; used by
  the Linux encoder), gated `cfg(any(test, not(windows)))`; Windows keeps its WIC path.
- `tests/encode_roundtrip.rs`, `tests/redaction.rs` - `#![cfg(windows)]` (they exercise
  WIC/COM and Windows paths; would not compile on other targets).
- `Cargo.toml` - `[target.'cfg(target_os = "linux")'.dependencies]` block (below).

### New dependencies (Linux-target only; zero reach into the Windows build)

| crate | version | purpose |
| --- | --- | --- |
| `x11rb` (features `["randr"]`) | 0.14.0 | pure-Rust X11 protocol client (no libX11/Xlib/bindgen) |
| `png` | 0.18.1 | PNG encode (replaces WIC off-Windows) |
| `ab_glyph` | 0.2.32 | annotation text rasterization |

Resolved Linux tree (`cargo tree --target x86_64-unknown-linux-gnu -e normal`, exit 0):
`ab_glyph{, _rasterizer, owned_ttf_parser, ttf-parser}`, `png{bitflags, crc32fast,
cfg-if, fdeflate, simd-adler32, flate2, miniz_oxide, adler2}`, `x11rb{gethostname,
rustix, linux-raw-sys, x11rb-protocol}` - all pure Rust, no C/system libraries, no
bindgen, no async runtime. Windows dependency resolution unchanged (`windows v0.62.2`
in lock). No `zbus` (tray deferred, plan S4 vs S7 conflict flagged), no `rfd`, no
`jpeg` encoder (Linux JPEG returns a clean error), no `libc`.

### Gates (exact commands and exit codes)

| command | exit | result |
| --- | --- | --- |
| `cargo check --target x86_64-unknown-linux-gnu` | 0 | zero warnings |
| `cargo check --tests --target x86_64-unknown-linux-gnu` | 0 | zero warnings |
| `cargo check --target aarch64-apple-darwin` | 0 | zero warnings |
| `cargo test` | 0 | **132 passed / 0 failed** (lib 122, encode_roundtrip 4, icon_asset 2, redaction 4; M0 baseline 114 -> +18 new) |
| `cargo clippy --all-targets` | 0 | **33 lib / 39 lib-test (33 dup) = exact M0 baseline** |
| `cargo clippy --target x86_64-unknown-linux-gnu --all-targets` | 0 | 11 lib / 17 lib-test after fixing the 4 M1 warnings (pre-fix 15/21); remaining all point at core files already inside the Windows baseline - **zero warnings in `src/platform/x11*`** |
| `cargo fmt --check` | 1 | exactly **228 diff locations = 228 M0 baseline**; every new file rustfmt-clean (per-file `rustfmt --edition 2024` on the 10 new files only, hand-formatted M1 lines in `config.rs`/`output.rs`); no repo-wide fmt |
| `cargo build --release` | 0 | see hash section; exe icon `.rsrc` byte-identical to packaged build |
| `rustup target add x86_64-unknown-linux-musl` | 0 | target installed |
| `cargo build --release --target x86_64-unknown-linux-musl` | **101** | all 19 crates incl. `x11rb`/`png`/`ab_glyph`/`nameshot` lib **codegen OK**; final ELF link fails `linker 'cc' not found` (host has no Linux/musl linker; no ELF produced, clean failure) |

Clippy fixes made this session (M1 files only): `x11/clipboard.rs` + `x11/display.rs`
`collapsible_if` -> edition-2024 let-chains, `x11/hotkey.rs` `manual_checked_ops` ->
`i.checked_div(per)` with `let-else` (semantics preserved: `per == 0` never returns a
keycode).

### Release exe hash: raw-SHA gate unachievable, resolved as diff-explained

M0's convention is "byte-comparable **or** diff-explained". Raw byte-comparability is
impossible on this toolchain, proven empirically:

1. Two forced rebuilds of **identical sources** (deleted `libnameshot-*.rlib/.rmeta`,
   unit fingerprints, and the exe; release build-script output preserved) produced
   `B19A06F9...1DE93` and `C68FE8A1...30B4D` - same file size (2,174,779), and a
   byte-for-byte comparison found **exactly 2 differing bytes**: offset **136** = PE
   COFF `TimeDateStamp`, offset **216** = OptionalHeader `CheckSum` (derived from it).
   Everything else - code, resources, imports - is byte-identical.
2. Therefore M0's `676AE653...D43E3` (2026-10-09) can never be re-struck by any later
   build, including M0's own rebuild; the raw SHA records its wall-clock link time.

Content parity with M0's final build, via COFF symbol/section comparison of the current
exe against the archived packaged exe `dist/NameShot-0.1.0-win-x64-20261009_181155.zip`
(`6A9B25E5...EF5C`), reproducing **exactly** the delta M0 documented for
`6A9B25E5 -> 676AE653`:

- File size delta **+21 bytes** (2,174,758 -> 2,174,779); all 11 PE section raw sizes
  byte-identical (`.text` 1,026,560, `.rsrc` 70,664 - **icon resource intact**).
- 5,985 function symbols in both; 5,526 unique names in both; 5,515 common names with
  **0 size differences**.
- Exactly 11 only-old / only-new pairs: 3 `.llvm.<hash>` local-symbol renames with
  identical sizes (160/32/80), 7 `overlay::edit` `EditSession` methods with the
  impl-block disambiguator shifted `NtB2_` -> `NtB5_` (the two M0-final `From` impls -
  same sizes), and `commit_text.llvm.*` (800/800). `EditSession::rebuild_bar`
  **3664 -> 3648** - the exact final value M0 documented for `676AE653`.
- COFF probes: `output::build_bmp` and `config::config_dir_from` **absent** from both
  exes; `output::build_dib` present and identical (1,264 bytes).

Combined with the 2-byte determinism result, the current exe's content equals M0's
`676AE653` build except that build's (unrecoverable) link timestamp. Current
`target\release\nameshot.exe` SHA-256: `C68FE8A13D476398ECD955C4B49F03267ACB85ABE20D7B566DCA006229330B4D`
(its same-content twin from minutes earlier: `B19A06F9...1DE93`).

Windows-impact elimination trail (why early M1 builds differed): `build_bmp` and
`config_dir_from` were codegen'd into the Windows exe (same codegen unit as the
referenced `build_dib`/`config_dir`), and the ungated `x11_logic` module shifted
rustc's codegen-unit partitioning of the whole crate - all now behind
`#[cfg(any(test, not(windows)))]`. A source-touch inventory (file mtimes since the M0
build) shows only the documented M1 files were modified. Environment audit: `rustc
1.99.0` unchanged, `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` empty, no `.cargo/config`,
release build-script output never re-run (no new `target\release\build\nameshot-*` dir;
windres object from the M0 icon build reused - confirmed by the byte-identical `.rsrc`).
Note: `cargo test`/dev-profile builds print `nameshot@0.1.0: no resource compiler
(windres/rc.exe) on PATH` - dev-profile only (test binaries, default icon, not
distributed); cosmetic, release unaffected.

### x11rb 0.14 API notes (for M1b event-loop work)

- Void requests return `Result<VoidCookie, ConnectionError>` (`?` + `.check()?`);
  reply requests: `?` then `.reply()?` - never `.and_then(|c| c.reply())`.
- `randr_get_monitors` (prefixed trait method), `put_image` arg order is
  `(format, drawable, gc, width, height, dst_x, dst_y, left_pad, depth, data)`,
  `send_event` takes the `SelectionNotifyEvent` struct directly +
  `EventMask::NO_EVENT` (no `Event::` wrapper exists).
- No `load_cursor`/`define_cursor`: `create_glyph_cursor` (mask char = glyph + 1) then
  `ChangeWindowAttributesAux::cursor`.
- No keysym->keycode request: hand-rolled scan of `get_keyboard_mapping` between
  setup `min_keycode`/`max_keycode`.
- `ImageOrder::MSB_FIRST`, `ModMask::from(u16)`, `GrabMode::ASYNC`,
  `PropMode::REPLACE`, `SELECTION_NOTIFY_EVENT: u8 = 31`.

### Not tested (no X11 session exists on this host)

Zero runtime verification: capture, hotkey grabbing, clipboard serving, windowing
primitives, zenity dialogs, display geometry - none executed (no X server, no WSL,
`DISPLAY` empty). Startup time, idle CPU, and memory on Linux are unmeasured. Tray
(zbus SNI vs plan's no-zbus constraint), accessibility, Linux JPEG, and the M1b
UI/event loop remain future work needing approval and an X11 environment.

### Stop point

M1 gates recorded above; awaiting review before any further phase (M1b runtime port,
Wayland/macOS/packaging).

---

## M1b - Linux X11 event-loop/runtime phase (2026-10-10 02:00-02:35)

**Scope accepted:** event-loop/runtime phase only; preserve the Windows build, packaged
`dist\nameshot.exe` (`6A9B25E5`) and all four ZIPs; no release package; no Wayland/macOS;
no support claims; if runtime testing is impossible, do only honestly-testable work, report
the exact limitation, and **stop rather than simulating a GUI run**. This section is a
separate reviewable record; it stops before any "Linux supported" claim.

### Step 1 - X11 runtime availability determination (result: NONE; checked before writing code)

| Check | Exact command | Result |
|---|---|---|
| WSL | `wsl.exe --status`, `wsl.exe -l -v` | `WSL_E_WSL_OPTIONAL_COMPONENT_REQUIRED` (not installed; install needs `wsl --install` + reboot + admin = out of scope) |
| Display env | `$env:DISPLAY`, `$env:WAYLAND_DISPLAY` | both empty |
| X servers | process scan `Xming\|vcxsrv\|X410\|Xorg\|Xephyr\|Xpra\|Xvfb\|weston\|cage\|mutter` | none running |
| X installs | `Test-Path` on VcXsrv/Xming/X410/MSYS2/Cygwin/docker paths | none (`C:\Program Files\Git\usr\bin` exists only) |
| X clients | `where.exe xeyes xwininfo xdpyinfo` | not found |
| ELF linkers | `where.exe cc gcc clang ld musl-gcc` | all NOT_FOUND |
| Toolchain | `rust-lld.exe` present at `lib\rustlib\x86_64-pc-windows-gnu\bin\` | exists, but an ELF cannot be *executed* on this host anyway |

**Conclusion (the exact limitation):** no X11 session and no way to launch a Linux binary
exist here. Runtime verification of the event loop is **impossible on this host**. Nothing
was simulated, no runtime support is claimed, and no GUI run was faked.

### What M1b implements (all new/changed files; compile-verified, never executed)

- `src/platform/x11_logic.rs` (+~430 lines, **x11rb-free, unit-tested**):
  - `SelectSession` + `SessEvent`/`SessOutcome` - pure selection state machine mirroring
    the Windows overlay policy (`on_down`/`on_move`/`on_up`/`release_action`/`on_key` in
    `overlay/mod.rs`): full-screen start, drag a band, release commits iff really moved and
    >= `theme::SPACE_XS` (4 px) both axes (sticky `moved` flag), click/sub-threshold band
    reverts to the drag-start selection, Enter commits current, Escape cancels.
    `band_rect`/`band_moved`/`cl` are line-for-line mirrors of the Windows helpers
    (duplicated, not moved, to keep the Windows exe's codegen untouched).
  - `veil_frame` + `compose_select` - pure scene composition mirroring `dim_frame`/`compose`:
    veil toward `theme::BG` at `VEIL_KEEP_PCT` (alpha forced opaque), crisp capture inside
    the selection, 1 px `SEL_OUTLINE` outside + 2 px `ACCENT` border. Resize handles and the
    size chip are **not** part of M1b.
  - `RawEvent`/`RawKind` + `to_session` - the event-mapping *policy*: primary-button filter,
    root->frame offset math, keycode vs `(escape, confirm)` comparison, `None` when the
    keymap is unavailable (the driver never invents a keycode).
- `src/platform/x11/eventmap.rs` (new, cfg linux): ~45 lines of mechanical x11rb field
  extraction (`ButtonPress`/`MotionNotify`/`ButtonRelease`/`KeyPress` -> `RawEvent`, reading
  `root_*` not `event_*` so delivery window is irrelevant). Compile-verified only.
- `src/platform/x11/app.rs` (new, cfg linux): the session driver - `App::connect` ->
  `grab_virtual_screen` -> overlay + veil/compose frame -> crosshair -> input focus ->
  `grab_pointer` -> blocking loop { `next_event` -> `pump_clipboard` -> `eventmap::extract`
  -> `to_session` -> `SelectSession::handle` -> redraw/commit/cancel } -> crop
  (`Frame::crop`) -> zenity `save_as` -> `encode_to_file` (PNG). Exit codes: 0 = saved /
  picker cancelled / Escape; 1 = error (reported on stderr). Compile-verified only.
- `src/platform/x11/hotkey.rs`: extracted `keycode_for_keysym(conn, keysym)` from the
  private `keycode_for` (same keymap scan, same errors); the driver resolves Escape/Enter
  through it - if the layout maps neither, keyboard shortcuts degrade to unavailable.
- `src/platform/x11/windowing.rs`: added `set_input_focus` (`InputFocus::POINTER_ROOT`) -
  the overlay is `override-redirect`, so no WM would ever focus it; without this, Escape/
  Enter would arrive nowhere.
- Wiring: `x11/mod.rs` adds `app`+`eventmap`; `platform/mod.rs` re-exports `app` on Linux;
  `lib.rs run()` split into `cfg(windows)` (untouched), `cfg(target_os = "linux")`
  (`platform::app::run()`), and `cfg(not(any(windows, target_os = "linux")))` (macOS keeps
  the honest "not ported" message). `Cargo.toml` net-zero (dev-dep experiment reverted).

**Deliberately out of scope (future phases):** move/resize handles, editing/annotation,
global-hotkey registration, clipboard-after-save, format/quality choice, tray, per-monitor
capture, multi-format save.

### The x11rb dev-dependency attempt (reverted; recorded because it is a real host limitation)

Trying to unit-test the x11rb event match directly, `[dev-dependencies] x11rb` made
`cargo test` fail on this host: x11rb -> `gethostname` + `rustix` -> `windows-sys` use
`#[link]` raw-dylib, so rustc synthesizes import libraries by spawning
`dlltool.exe -d ... --temp-prefix kernel32.dll:`. Empirically isolated:

- the toolchain *ships* dlltool at `...\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\dlltool.exe`
  (binutils 2.44, runs standalone: `--version` OK) but rustc's unaided lookup reports
  "program not found";
- with that dir on `PATH`, dlltool runs but fails `CreateProcess` on the rustc-style
  `--temp-prefix kernel32.dll:` (colon) even with `rm.exe`/`sh.exe` on `PATH`; a normal
  prefix succeeds only with `rm` available. The import libs are written, exit code is 1.

Rather than depending on PATH surgery or a MinGW install (both environment changes),
the mapping policy was moved **into** the x11rb-free core (strictly better: it now runs in
the regular Windows suite) and the dev-dependency was reverted. `Cargo.lock` is unchanged
(SHA-256 `5EBD9871B52F022FF3234E32690AC96447B691B56639B989D74C35324591C75E`, x11rb was
already a Linux-target package). Consequence: `x11/eventmap.rs`'s field extraction is
compile-verified only (documented in its header).

### External formatting wave (not from this session - reported, not hidden)

At **02:01:23-24** (between this session's first per-file `rustfmt` on 7 M1b files and its
first `cargo check`), ~30 previously-unformatted files under `src/**` were rewritten
(`annotate, app, capture, config, editor/*, encode, error, flatten, hotkey, image/*, os,
output, overlay/{mod,edit}, settings_ui, strings, text, theme, win/*`); `tests/` was NOT
touched. No command in this session formatted them (only explicit per-file `rustfmt
--edition 2024 <7 M1b files>` and non-writing `cargo fmt --check` were run; `build.rs` is
icon-only). Effect: the M1-recorded 228-hunk fmt baseline collapsed to **1 hunk**.
The wave is formatting-only (proven below: all code bytes identical).

### Gates (exact commands, exit codes)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Windows check | `cargo check` | 0 | ok |
| Unit suite | `cargo test` | **0** | **148 passed / 0 failed** = lib **138** (M1: 122; +16 M1b) + encode 4 + icon 2 + redaction 4 (M1 total 132 -> 148) |
| Linux check | `cargo check --target x86_64-unknown-linux-gnu` | 0 | ok |
| Linux tests check | `cargo check --target x86_64-unknown-linux-gnu --tests` | 0 | ok |
| macOS check | `cargo check --target aarch64-apple-darwin` | 0 | ok |
| Windows clippy | `cargo clippy --all-targets` | **0** | **33 lib / 39 test = exact M1 baseline** (0 new warnings; 2 found in M1b test/code and fixed: `as_chunks` lints) |
| Linux clippy | `cargo clippy --all-targets --target x86_64-unknown-linux-gnu` | **0** | **11 lib / 17 test = exact M1 baseline**; zero warnings in `platform/x11*` |
| Fmt | `cargo fmt --check` | 1 | **1 hunk: `tests\encode_roundtrip.rs:11`** (import order) - pre-existing M1-era hand-format debt, untouched by M1b; all M1b files rustfmt-clean |
| Release | `cargo build --release` | 0 | ok; build-script cache hit (no windres re-run; icon `.rsrc` reused) |
| musl | (unchanged from M1) | 101 | same environmental blocker: no `cc`/ELF linker on host; not re-attempted beyond M1's full record |

New tests (16, all in the Windows suite): full-screen start; drag commit (inclusive-point
rect sizes); drag from either corner normalizes; plain click reverts; 3x3 sub-threshold
reverts / 4x4 commits (mirrors the Windows `release_action` tests); sticky-`moved` commit
even when the band ends back at the previous selection; band clamped to screen; no-drag
input ignored; Enter commits / Escape cancels + clears the drag; veil math vs `theme::BG`/
`VEIL_KEEP_PCT` with input immutability + alpha forced; compose border/crisp/veil placement
with input immutability; full-screen-compose edge clipping; mapping: press/move offsets,
primary-button-only, Escape/Enter/keymap-absent.

### Windows preservation proof (byte-level, against a copy saved before any M1b change)

Saved copy `C68FE8A13D476398ECD955C4B49F03267ACB85ABE20D7B566DCA006229330B4D`
(2,174,779 bytes) -> rebuilt `2E347AD29793E6B12B8B2E910387C1EF7B0AA2869290B881CAC4D8279ED2AC4D`
(2,174,800, +21). Content proof (COFF differ + per-section byte compare):

- `.text` (all code): **bytes identical** (1,026,560). `.data`, `.pdata`, `.xdata`,
  `.idata`, `.reloc`: **bytes identical**. `.rsrc` (icon): **bytes identical** (70,664).
- `.rdata`: same size; 175 changed u32 slots whose values move by small deltas
  (341->348, 265->268, ...) = file:line/col of `panic!` `Location` structs - pure
  line-number churn from the external formatting wave.
- Symbols: 5985 total, 0 size-diffing common symbols, 21 "renames" = same functions with
  different `.llvm.<hash>` suffixes (span-derived module hashes). Probes unchanged:
  `build_bmp`/`config_dir_from` absent, `build_dib` present (1264).
- Net: **zero code change**; the +21 is the COFF string-table/symbol-hash + the known 2
  volatile bytes (PE `TimeDateStamp`, `CheckSum`). M1b's own sources contribute nothing to
  the Windows exe (everything new is `cfg(test)`/`cfg(target_os = "linux")`).
- Untouched: `dist\nameshot.exe` `6A9B25E5...A32EF5C`; 4 ZIPs (mtimes 10/07-10/09);
  PID 9420 still running; `assets/icon.ico` unchanged.

### Not tested / limitations (exact)

- **No runtime execution of anything X11** - no X server, no WSL, no ELF execution path
  (see Step 1). The driver, windowing, capture, clipboard, dialogs, encoding-on-Linux have
  **never run**; no startup/idle/CPU/memory numbers; no claim of Linux support.
- `x11/eventmap.rs` field extraction is compile-verified only (dev-dep reverted, see above);
  the mapping decisions behind it are unit-tested.
- Linux JPEG still unavailable (M1 decision); musl/ELF link blocked by missing `cc`.
- The single fmt hunk (`tests/encode_roundtrip.rs:11`) is pre-existing and left alone.

### Stop point

M1b gates recorded above. **Stopping here for review** before: claiming any Linux/X11
runtime support, running a manual X11 QA session, starting M1c (handles/editing/tray), or
touching Wayland/macOS/packaging. Note for the reviewer: an external formatting wave
touched `src/**` at 02:01 (not from this session) - see its subsection above.

---

## M1b-v - Reproducible Linux/X11 validation path + host investigation (2026-10-10 03:00-03:45)

**Scope (reviewer directives):** prepare a reproducible validation path using a real Linux
environment with an X server; determine whether the dlltool issue is specific to this build
host; verify the X11-dependent code in the appropriate Linux toolchain; do **not** remove or
replace dependencies to make the current host pass; record exact environment/commands/exits;
separate compile checks from live checks (capture, selection, hotkeys, clipboard, window
lifecycle); claim no behavior that was not run; preserve artifacts; summarize the external
formatting wave and confirm/deny non-formatting changes; no Wayland/macOS/packaging; stop.

### 1. External formatting wave audit - conclusion: NO non-formatting changes

**Method (reproducible, no VCS needed):** every `panic!` site embeds a `Location`
`{file, line, col}` as read-only data in `.rdata`. The pre-wave build (the saved copy
`C68FE8A1…`, built before the wave) and the post-wave build (`2E347AD2…`) were diffed at
byte level: 175 changed u32 slots in `.rdata`, decoded back to file/line/col entries
(pointer VA minus image base `0x140000000`, converted through the section VirtualAddress;
decoder: `%TEMP%\opencode\locdiff.ps1`). Decode result: **160 unique changed Location
entries, 0 undecodable slots.**

**Result:** changed spans exist in exactly **12 files, every one a member of the 02:01 wave**
(`annotate, app, editor/mod, editor/toolbar, flatten, image/mod, output, overlay/edit,
overlay/mod, settings_ui, text, win/tray`). **Zero unexpected files**: no entry from
`tests/*` (mtimes still 12:28, pre-wave), none from this session's M1b files
(`platform/mod.rs`, `lib.rs`, `platform/x11*`). Delta signatures are pure formatting:
cumulative monotonic line shifts (+3…+252 down the file), column rewraps
(e.g. `872:32→963:18`), one line *merge* (`output.rs 117:22→116:76`), and the largest shift
`settings_ui 339→507 (+168)` verified by inspection as the 17-tuple `items` array rustfmt
exploded to one tuple per line (read at `settings_ui.rs:293-342`).

**Corroborating evidence already recorded:** `.text`, `.data`, `.pdata`, `.xdata`, `.idata`,
`.reloc`, `.rsrc` are **byte-identical** between the pre-wave and post-wave exes → zero
semantic change in any Windows-compiled code; clippy counts unchanged (33/39); test counts
unchanged except this project's own additions (132 → 148 = +16 M1b, +8 now Linux-gated).

**Residual limit (stated, not hidden):** with no VCS on this repo, comment/doc-only edits
inside wave files cannot be fully excluded; they do not affect behavior or the binary.
Recommendation (not executed): establish a git baseline so future audits are trivial.

### 2. dlltool / x11rb verification limitation - verdict: SPECIFIC TO THIS BUILD HOST

No dependency was removed, replaced, or added this session to make this host pass:
`Cargo.toml` untouched (mtime 2026-10-10 02:08:33 = M1b's accepted revert), `Cargo.lock`
still `5EBD9871…C75E`, x11rb remains M1's Linux-target dependency. Investigation only
(temp probe project under `%TEMP%\opencode\dlltool_probe\`, logs `phaseA-F.log`; nothing on
the repo or toolchain was modified).

**Environment captured (phaseA):** rustc 1.99.0 (b940084d7 2026-09-28), host
`x86_64-pc-windows-gnu`, single toolchain `stable-…` installed 2026-10-06 22:36;
`self-contained\dlltool.exe` = GNU binutils 2.44, mtime 2026-10-06 22:23; the self-contained
dir is **not** on PATH (22 PATH entries, Git has only `\cmd`); no MSYS2/MinGW anywhere;
`rustup component list` = `rust-mingw-x86_64-pc-windows-gnu (installed)`.

**Reproduction matrix (isolated crate + `x11rb = "0.14"` dev-dep, `cargo test --no-run`):**

| Probe | PATH | Exit | Verbatim key line |
|---|---|---|---|
| P0 empty crate | default | **0** | baseline builds fine |
| P1 x11rb chain | default | **101** | `error: error calling dlltool 'dlltool.exe': program not found` (fails in `gethostname` + `windows-sys`) |
| P2 + self-contained dir | +`…\bin\self-contained` | **101** | `dlltool could not create import library with …dlltool.exe -d …\kernel32.dll_imports.def -D kernel32.dll -l …lib -m i386:x86-64 -f --64 --no-leading-underscore --temp-prefix kernel32.dll:` then `…dlltool.exe: CreateProcess` |
| P3 + Git `\usr\bin` (sh/rm) | +above | **101** | same CreateProcess failure (helpers present do not help) |

**Root cause (dlltool `--verbose`, verbatim):**
`Using file: …\self-contained\as` → `run: …\self-contained\as --64 -o …_lib_h.o …_lib_h.s`
→ `No such file or directory` → `CreateProcess`. dlltool builds import libraries by
spawning an assembler **next to itself**; **that `as` does not exist** (a sysroot-wide search
finds only `ld.exe`; `-f --64` decodes as `--as-flags --64`, per `dlltool --help`).

**Not a corrupted install:** `manifest-rust-mingw-x86_64-pc-windows-gnu` lists exactly
`self-contained/{dlltool.exe, ld.exe, x86_64-w64-mingw32-gcc.exe}` (+libwinpthread,
GCC-WARNING.txt) - all present. **rust-mingw ships no assembler**, and rustc (this version)
passes no `-S`, so the stock pair cannot complete here.

**Why it worked on 2026-10-07:** `target\release\deps\rustc4gUHyK\{api-ms-win-core-handle-
l1-1-0, kernel32, ntdll, oleaut32, user32}.dll_imports.lib` are dated **2026-10-07 17:48** -
rustc's dlltool run *succeeded* with this same toolchain then, i.e. a different, working
dlltool+as pair was resolvable that day (e.g. a MinGW/MSYS dlltool on PATH) and is **gone
now**. Environment changed after 10-07 → the failure is a **current-state property of this
host**, not a NameShot/x11rb code defect and not reproducible as "always broken here".

**Blast radius:** only *new/changed* raw-dylib crates for windows-gnu (x11rb's
gethostname/windows-sys chain). NameShot's own Windows build is unaffected (import-lib
cache + fingerprint reuse; this session's `cargo build --release` exit 0, exe unchanged
`2E347AD2…`). x11rb is Linux-target scoped and never compiled for Windows here.
**The Linux validation path is unaffected**: dlltool/`as` are PE import-lib machinery and are
never invoked on a Linux toolchain.

**Open question (not claimed):** whether a second stock windows-gnu host reproduces P1/P2
cannot be determined from this host (none available). Proposed host remediation (NOT
executed, would need approval; irrelevant to the Linux path): put a working MinGW dlltool+as
pair on PATH or restore an `as` beside the self-contained dlltool.

### 3. X11-dependent code in the Linux toolchain (compile tier, executed here)

- **New: 8 Linux-gated unit tests** in `src/platform/x11/eventmap.rs`
  (`#[cfg(all(test, target_os = "linux"))]`): root-vs-event coordinate reading, button/motion/
  key field extraction, non-primary extraction is *not* filtered (policy owns that), expose→
  `None`, full wire→`to_session` path with origin offset, and policy rejection of non-primary/
  unresolved keys. Types verified against the extracted `x11rb-protocol-0.14.0` sources.
- `cargo check --target x86_64-unknown-linux-gnu --tests` → **exit 0** (compiles on the Linux
  toolchain). **Not executed** - no Linux host here; execution belongs to the harness (T2).
- Expected on a real Linux run: lib tests **146** (138 cross-platform + 8), suite total
  **156**.

### 4. Reproducible validation path (deliverable, not run here)

`scripts/linux_x11_validate.sh` - one command for a real Linux+X11 host. **T0** environment
capture (uname/os-release/rustc/DISPLAY/X server/required tools, each with exit codes);
**T1 compile tier** (`cargo check --all-targets`, `clippy --all-targets`, `fmt --check`
recorded with the expected exit-1 note, `build --release`); **T2 unit tier** (`cargo test`
with the 156 expectation); **T3 live tier**, each behavior separate with exact commands,
artifacts (all under a unique `${TMPDIR:-/tmp}/nameshot-validation.XXXXXX` evidence dir:
`validation-results-<ts>.log`, `*.xwd`, wintree, app stdout/stderr - never the repository,
never deleted) and **MANUAL
RESULT** lines the operator must fill: **T3.1 window lifecycle** (map/focus/grab/Escape-exit0),
**T3.2 capture** (grab visible in overlay, screenshot compare), **T3.3 selection** (XTEST
drag → band screenshot, Enter → save path with zenity present/absent expectations, click-
revert sub-check), **T3.4 hotkeys** (global registration **NOT IMPLEMENTED in M1b** - no
check exists; Escape/Enter exercised in T3.1/T3.3), **T3.5 clipboard** (clipboard-after-save
**NOT IMPLEMENTED in M1b**; served-content verification **NOT VERIFIED/deferred**). Honesty
rules are in the header: the script never claims a pass; compile tiers prove nothing about
live tiers; the log must be attached to BUILD_LOG before any runtime statement. Exit codes:
0 machine tiers ok / 1 machine-tier failure / 2 guard refusal / 3 prerequisite missing
(offline cache and/or T3 toolset incl. Xvfb). The harness is offline by default
(`CARGO_NET_OFFLINE=true` + `--offline`), sends automated input only to its own private
Xvfb, and invokes no clipboard tool.

**Verification of the harness itself on this host (safety rev R1):** `bash -n` on both
scripts -> **exit 0**; `bash scripts/linux_x11_validate_tests.sh` -> **exit 0, 62/62
assertions** (guard refusals included: test mode / non-repository / non-Linux each exit 2
and create nothing - details in section 7). The harness main flow has **never been
executed** (no Linux here, reviewer directive) - first execution is on the validation host.

**Compile vs live status matrix (nothing claimed beyond evidence):**

| Behavior | Compile evidence (this host) | Live check | Status now |
|---|---|---|---|
| capture | linux `check`/`clippy` exit 0 | T3.2 | **NOT RUN** |
| selection | 146-to-be unit tests + linux `check` exit 0 | T3.3 | **NOT RUN** |
| hotkeys | n/a (global registration not implemented in M1b) | T3.4 | **NOT IMPLEMENTED** |
| clipboard | pump path compiles (linux `check` exit 0) | T3.5 | **NOT VERIFIED (deferred)** |
| window lifecycle | `app.rs` compiles (linux `check` exit 0) | T3.1 | **NOT RUN** |

### 5. Gates this session (exact commands, exit codes)

| Gate | Command | Exit | Result |
|---|---|---|---|
| Windows check | `cargo check` | 0 | ok |
| Unit suite | `cargo test` | 0 | **148 passed** (138+4+2+4; unchanged - Linux-gated tests skipped) |
| Windows clippy | `cargo clippy --all-targets` | 0 | **33 lib / 39 test = exact baseline** |
| Fmt | `cargo fmt --check` | 1 | the known single hunk `tests\encode_roundtrip.rs:11` (M1-era, untouched) |
| Linux tests compile | `cargo check --target x86_64-unknown-linux-gnu --tests` | 0 | includes the 8 new eventmap tests |
| Linux clippy | `cargo clippy --all-targets --target x86_64-unknown-linux-gnu` | 0 | **11 lib / 17 test = exact baseline** |
| macOS check | `cargo check --target aarch64-apple-darwin` | 0 | ok |
| Release | `cargo build --release` | 0 | cached; windows-only code untouched |
| eventmap fmt | `rustfmt --edition 2024 src\platform\x11\eventmap.rs` | 0 | ok |
| Harness syntax | `bash -n scripts\linux_x11_validate.sh` | 0 | ok |
| Harness guard | run on MINGW | 2 | refuses non-Linux, creates nothing |

Note (2026-10-10): the two harness rows above were run against safety rev R0; rev R1
re-verification (syntax + safety suite, harness never executed) is section 7.

### 6. Preservation (verified after all work)

`dist\nameshot.exe` = `6A9B25E5…A32EF5C`; 4 ZIPs untouched (mtimes 10/07-10/09); PID 9420
was running at that verification (since then the host rebooted 2026-10-10 13:18:16, the
process is gone, and it was NOT restarted - reviewer directive, re-verified 2026-10-10);
`Cargo.lock` = `5EBD9871…C75E`; target exe unchanged `2E347AD2…`; the new repo files are
exactly `scripts/linux_x11_validate.sh` and `scripts/linux_x11_validate_tests.sh`
(re-verified 2026-10-10); probe artifacts live only under `%TEMP%\opencode\`. Exception
flagged, NOT removed (approval required): two pre-existing debris files at repo root,
both predating M1b-v and untouched by the safety revision - `z` 444 B @2026-10-10 01:11:13
(an old exe-symbol probe output) and `kernel32.dll` 0 B @2026-10-10 02:06:43 (name matches
the dlltool `--temp-prefix` experiment recorded in section 2; provenance not re-established
this session).

### 7. Harness safety rev R1 - implemented and verified (2026-10-10; harness NOT executed)

**Reviewer directive:** harness-only safety revision - isolated Xvfb only for automated
input, offline Cargo by default, bounded timeouts, scoped cleanup, unique temporary output,
no clipboard access, non-interactive tests for these safeguards; run only syntax checks and
those tests; do not execute the harness; this log updated only with what is implemented and
verified; stop for review.

**Files (exactly two new repo files):**

| File | Lines | SHA256 (after verification) |
|---|---|---|
| `scripts/linux_x11_validate.sh` (rev R1) | 610 | `08EE19C8B05063BD…` |
| `scripts/linux_x11_validate_tests.sh` (new) | 250 | `BA928FDC72E2FC19…` |

Out-of-repo under `%TEMP%\opencode\m1b_v2\`: rev R0 `linux_x11_validate.sh.old`
(271 lines, `0403AD6D932B8AE1…`) and full diffs `harness.diff`, `tests.diff`.

**Requirement -> mechanism -> test coverage** (harness line numbers):

| Requirement | Mechanism | Tests |
|---|---|---|
| Isolated Xvfb for automated input | `nsh_gui_input_allowed`/`nsh_require_gui_input` gate `run_x` (:134-150, :197-204) and `nsh_start_app` (:362-379); ambient DISPLAY recorded then `unset` (:146-150, :422); private Xvfb `-nolisten tcp` (:310-340); T3 toolset missing -> prerequisite report, exit 3, never a real-desktop fallback | 22-23, 34-41, 56-59 |
| Offline Cargo by default | `CARGO_NET_OFFLINE=true` (:405); `--offline` on check/clippy/build/test/metadata (:346, :448-474, :498); `cargo metadata --offline` preflight (:343-359) | 16-17, 60-62 |
| Bounded timeouts | `timeout -k` on every command (:184, :191); `nsh_wait_bounded` app waits (:209-221); SIGTERM -> bounded grace -> SIGKILL on own pids only (:224-237); Xvfb socket wait bounded (:326) | 18, 42-49 |
| Scoped cleanup | `trap` EXIT/INT/TERM -> `nsh_cleanup`, touching exactly `NSH_APP_PIDS` + `NSH_XVFB_PID`; evidence never deleted (:241-280, :406-408) | 21, 50-51 |
| Unique temporary output | `mktemp -d …/nameshot-validation.XXXXXX` for log, app stdout/stderr, wintree, screenshots (:400-404) | 19-20 |
| No clipboard access | no clipboard tool anywhere; `CLIPBOARD: NOT TESTED` / `GLOBAL HOTKEYS: NOT TESTED` markers (:577-583) | 1-12, 24-26 |
| Non-interactive tests | suite sources the harness (`BASH_SOURCE` guard :608-610) and never runs `main`; guard refusals exercised in subshells | whole suite (esp. 28-33) |

**Verification actually run on this host (nothing else executed):**

| Check | Command | Exit | Result |
|---|---|---|---|
| Harness syntax | `bash -n scripts/linux_x11_validate.sh` | 0 | ok |
| Tests syntax | `bash -n scripts/linux_x11_validate_tests.sh` | 0 | ok |
| Safety suite | `bash scripts/linux_x11_validate_tests.sh` | 0 | `1..62`, `PASS: 62/62` |

**Harness exit-code contract (documented, unexecuted):** 0 machine tiers ok / 1 machine-tier
failure / 2 guard refusal (not Linux / not NameShot root / `NSH_TEST_MODE=1`) / 3
prerequisite missing (offline cache and/or T3 toolset incl. Xvfb), with blocked tiers
recorded NOT RUN and no GUI input attempted.

**Status:** the harness main flow has never been executed; no GUI input, no cargo
invocation through the harness, no network, no installs, no privileged action; NameShot not
restarted (PID 9420 gone since the 2026-10-10 13:18:16 reboot, verified, reviewer
directive). Stopping for review.

### Stop point

M1b-v recorded above. **Stopping for review** before: any execution of the validation harness
(needs a real Linux+X11 host), any toolchain/PATH repair on this host (proposal only), any
runtime/support claim for Linux/X11, any second-platform work (Wayland/macOS), or packaging.
## Phase 7 -- Perf pass + clean UI refresh (2026-10-12)

### Optimization (capture speed, no lag, low resources)
- Cargo.toml: new [profile.release] (strip, lto, codegen-units=1; keep opt-level=3/panic=unwind). Release exe: 2135 KB -> 604 KB (-72 %).
- capture.rs: single alloc + copy_nonoverlapping + set_len; alpha fix as u32 OR 0xFF000000 (4x fewer stores, vectorizes).
- overlay/mod.rs dim_frame: compile-time LUTs (no mul/div per pixel) + lut_matches_the_blend_formula test.
- flatten.rs Preview::render: draft path refreshes only dirty rows (union of old/new draft bounds; text drafts take the full-refresh path); copy_rows + preview_dirty_draft_matches_full_flatten test.
- Pen/shape drags repaint the draft-bounds union only (overlay/edit.rs, editor/mod.rs); pen draft rebuilt only when the stroke grows (no per-mousemove Vec clone).
- Cached: 9 stock cursors (win::gdi::system_cursor, OnceLock), WIC factory (thread-local), text HFONTs per size (thread-local).
- Startup: title font deferred to first About open (OnceLock); per-keystroke String alloc in the message prefilter replaced by zero-alloc compare.
- Full suite green (148 lib + 4 + 2 + 4 integration), zero warnings.

### UI refresh (modern / clean / minimalist, whole chrome)
- theme.rs: electric-blue accent #2F8FFF (+hover/pressed), lifted surfaces, hairline borders, RADIUS 4 -> 8. All WCAG AA theme tests green.
- Rest button faces are borderless; hairline appears on hover/selection. Panels, chips, Settings/About pick it up automatically (no geometry change).
- docs/UI_REFRESH_SPEC.md section 2 palette/states updated to match.

### Verification run on this host
- cargo test --offline: 148 + 0 + 4 + 2 + 4, all ok, no warnings.
- scripts/make-release.ps1 -SkipInstaller: release OK (5m55s), dist/NameShot-0.1.0-win-x64-20261012_170223.zip (427 KB).
- Smoke: exe starts (tray PID), killed cleanly.
