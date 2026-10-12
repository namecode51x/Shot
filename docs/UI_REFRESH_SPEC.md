# NameShot — UI Refresh Specification

Scope: a visual-only refresh of every NameShot-owned surface, implemented in the
existing Rust/Win32/GDI architecture. No new runtime, no timers, no animation,
no change to capture/annotation behaviour, close policy, settings semantics or
accessibility names. Design tokens live in one module: `src/theme.rs`.

## 1. Surfaces inventory (before)

| Surface | Where | Current look |
|---|---|---|
| Capture overlay — veil | `overlay/mod.rs` `dim_frame` | half-brightness multiply of the frozen frame |
| Selection frame + handles | `overlay/mod.rs` `compose` | 1 px black outer + 1 px teal accent, eight 8×8 accent squares |
| Selection hint strip | `overlay/mod.rs` `compose` | 48 px `BG_ALT` box, two centered lines, bottom center |
| Size chip (selecting) | — | **missing** (dimensions shown only in the editing status chip) |
| Tool rail (editing) | `overlay/edit.rs` `rebuild_bar` | text-labelled buttons, teal selected fill, floating over the veil |
| Action strip (editing) | `overlay/edit.rs` `rebuild_bar` | same button style; Copy/Save/Close all equal weight |
| Status chip (editing) | `overlay/edit.rs` `compose` | `BG_ALT` box, secondary text; hover tips + transient messages |
| Settings | `settings_ui.rs` | dark rows, no grouping, owner-drawn OK/Cancel |
| Hotkey capture | `settings_ui.rs` | same |
| About | `settings_ui.rs` `paint_about` | flat dark text block |
| Errors | `overlay/edit.rs`, `app.rs` | native message boxes everywhere |
| Tray menu | `win/tray.rs` | native OS menu (kept) |
| Tray icon | `win/tray.rs` | dark rounded square + accent crop brackets (kept, follows accent token) |
| Legacy editor window | `editor/mod.rs` | shares `toolbar.rs`; not reachable from the flow (kept coherent) |

Scattered magic values (the problem this refresh fixes): `win/dark.rs`
(`BG`, `BG_ALT`, `CANVAS`, `FG`, `FG_DIM`, `SEL_BG`, teal `ACCENT_RGB`),
`editor/toolbar.rs` (`BTN_BG`, `BTN_BG_HOVER`, `BTN_BG_DIM`, `BTN_BORDER`,
`TEXT_ON_ACCENT`, pressed colors), `overlay/mod.rs` (`ACCENT`, black frames),
`overlay/edit.rs` / `editor/mod.rs` (canvas border `0x000E_1012`).

## 2. Design tokens (final)

All colors are `0x00BBGGRR` COLORREFs produced by `theme::cref`. Values are
also given as hex RGB for review.

### Palette

| Token | Hex | Use |
|---|---|---|
| `BG` | `#101216` | main background (overlay veil blend target) |
| `SURFACE` | `#1A1F27` | primary surface: rail/strip panels, chips base, Settings/About/window class fill |
| `ELEVATED` | `#232833` | elevated surface: button hover, inputs, status chip |
| `CANVAS` | `#0C0D10` | area behind the fitted image |
| `BORDER` | `#2E3440` | 1 px hairline: hover, selection and disabled faces only (rest faces are borderless) |
| `BORDER_INPUT` | `#3A4251` | input/control border (edits, value box) |
| `FG` | `#F3F5F8` | primary text |
| `FG_DIM` | `#A4ACB9` | secondary text, hints, status chip |
| `FG_DISABLED` | `#6E7581` | disabled labels |
| `ON_ACCENT` | `#0C1220` | text/icons on accent fill |
| `ACCENT` | `#2F8FFF` | one vivid electric blue: active tool, primary action, focus rings |
| `ACCENT_HOVER` | `#5BA8FF` | accent hover |
| `ACCENT_DOWN` | `#1F6FD6` | accent pressed |
| `HOVER` | `#292F3B` | hover lift above `ELEVATED` |
| `PRESSED` | `#12151A` | quiet button pressed |
| `OK` | `#4CC38A` | success status only |
| `WARN` | `#E7B14C` | warning status only |
| `ERROR` | `#E56A6A` | error status / destructive only |
| `SEL_OUTLINE` | `#05070A` | near-black under the selection outline |
| `VEIL_KEEP` | 40 % | frozen-frame pixel share kept outside the selection; the rest blends toward `BG` (uniform near-black veil) |

### Type

| Token | Value |
|---|---|
| Font family | Segoe UI, fallback stock `DEFAULT_GUI_FONT` (unchanged) |
| `FONT_UI_PX` | 16 (controls, buttons) |
| `FONT_SMALL_PX` | 13 (hints, chips, helper text) |
| `FONT_TITLE_PX` | 22, weight semibold (About window title only) |
| Scale | title 22 / ui 16 / small 13 — no essential label below 13 px |

### Spacing / shape

| Token | Value |
|---|---|
| `SPACE_XS/S/M/L` | 4 / 8 / 12 / 16 px |
| `RADIUS` | 8 px (button faces, chips, panels — GDI `RoundRect`) |
| `BORDER_W` | 1 px (all borders) |
| `ICON_PX` | 18 px icon box, 2 px stroke, optical center |
| Rail/strip geometry | `PAD=8`, `RAIL_W=60`, `RAIL_GAP=4`, `STRIP_H=46`, chip `H=22` — kept from the tested coordinate model |

### States (never color-only)

| State | Cue |
|---|---|
| Rest | flush with panel, borderless (clean look) |
| Hover | one-step lift + 1 px `BORDER` hairline |
| Pressed | `PRESSED` fill, borderless (sunken) |
| Selected (tools) | `ACCENT` fill, `ON_ACCENT` icon, **plus a 3 px indicator bar** on the rail edge (shape cue, survives grayscale) |
| Selected (colors/widths) | 1 px `FG` outline (existing shape cue) |
| Disabled | `FG_DISABLED` label/icon, `PRESSED` face |
| Keyboard focus | 1 px `ACCENT` focus ring (only while keyboard mode is on) |

### Actions

| Action | Treatment |
|---|---|
| Copy | **primary**: `ACCENT` face, `ON_ACCENT` label |
| Save | **secondary**: borderless `ELEVATED` face, `FG` label |
| Close | **quiet**: flush face, `FG_DIM` label, `BORDER` on hover |

## 3. Surface plans

### Capture overlay (Phase 2)

- **Veil**: `dim_frame` blends toward `BG` keeping 40 % of each pixel
  (one O(n) pass at open/restart only — same cost class as today's `>>=1`).
- **Selection**: `SEL_OUTLINE` 1 px outer + 2 px `ACCENT` inner line. Handles:
  corner handles 8×8 `ACCENT` with `SEL_OUTLINE` edge; edge handles compact
  bars (N/S 14×4, E/W 4×14) — hit tolerance (`HIT=7`) unchanged.
- **Size chip (new)**: compact `ELEVATED` chip with `W × H` in `FG_DIM`,
  placed below the selection at its left edge; flips above near the bottom
  screen edge; drawn only in the selecting phase. Selecting-phase region
  invalidation expands from 10 to 32 px so the chip is covered.
- **Hint strip**: same geometry, `SURFACE` + `BORDER`, line 1 `FG`, line 2
  `FG_DIM`, `RADIUS` corners.
- **Rail**: `SURFACE` panel with 1 px `BORDER` behind the buttons; tool buttons
  become **icon-only** (`Content::Icon`) with the existing status-chip
  tooltips; states per §2. Geometry comes from the adaptive planner (§3
  Editor chrome placement): left else right else clamped, height shrinks
  when the selection is short.
- **Strip**: `SURFACE` panel; Undo/Redo become icons; Copy primary, Save
  secondary, Close quiet. Colors/widths keep swatch/dot treatment with token
  outlines. Placement via the adaptive planner: below → above → pinned,
  rejected whenever it would intersect the rail or chip.
- **Status chip**: `ELEVATED` + `BORDER`; `FG_DIM` text; **error state** =
  `ERROR` text + `ERROR` border, set for copy/save failures (see §3 Errors).
- **Tooltips**: unchanged mechanism (chip-only, no floating windows, no
  timers); all icon-only actions already have tips — covered by the existing
  `tips_cover_every_button_with_shortcuts_only_where_wired` test.
- Frozen frame, overlay HWND, backbuffer `BitBlt` path, region-limited
  invalidation, capture route and close policy: untouched.

### Editor chrome placement (adaptive)

- Pure module `src/overlay/layout.rs::plan(&Measures) -> Layout`, called
  only from `rebuild_bar` (state/selection/client change — no polling, no
  timers): **measure → generate candidates → reject → rank → mode**.
- Modes: **Expanded** = today's metrics (button height `th_max`, normal
  rail gap) whenever it ranks first; **Compact** = short selections (the
  reported ~330×207 case): `th` sweeps `th_max → 20` at the normal rail
  gap, strip x shifts clear of the rail column, strip band stays
  below/above/pinned; **EdgeFallback** = degenerate or pinned cases (or no
  clear placement at usable size): floor `th` down to 14 with the rail gap
  allowed to shrink to 0, rail and strip additionally pinned to the
  screen edge.
- Candidates cover rail left/right/clamped, strip below/above/bottom-pinned,
  strip x centered/shifted, chip side flips; **any candidate intersecting
  the client bounds or another chrome rect (rail/strip/chip) is rejected**
  — panels can no longer paint through each other.
- Ranking (two phases): phase one accepts only usable-size buttons
  (`th >= 20`, the Compact minimum) placed fully clear of the selection,
  ranked by minimum movement — so tiny selections (e.g. 50x50) no longer
  collapse the rail to the 14 px floor just to shave movement; phase two
  (floor metrics, edge pins, covering placements) runs only when nothing
  clear fits. Invariants asserted by 10 unit tests: all rects
  inside the client, rail/strip/chip pairwise disjoint, chip exactly
  `strip.top-4`/`strip.bottom+4`.
- The pre-existing contract test (rail icon-first + edge-aware, strip
  below→above, chip docking ±4) still passes unchanged.

### Settings / Hotkey / About (Phase 3)

- One shared layout language: group headers painted in the window `WM_PAINT`
  (`FONT_SMALL`, `FG_DIM`) with a 1 px `BORDER_INPUT` divider under each —
  groups: **Capture & Hotkey**, **Save & Format**, **Startup** (matches the
  existing controls; no settings added/removed).
- Background moves from `BG` to `SURFACE` (class brush + `ctl_color` bk).
- Row rhythm on a 4 px grid; helper text stays `FG_DIM` at 13 px.
- Checkboxes/edits/combos opt into `DarkMode_Explorer` (`dark_control`,
  currently defined but never called) so glyphs match the dark surface.
- Buttons keep the owner-drawn `paint_face` path, restyled from tokens
  (OK = primary).
- About: `SURFACE` fill (matches the class brush), title in the new title
  font, short `ACCENT` divider, version/description/privacy/OS on the type
  scale; same Close button.
- Accessibility names, control ids, pending-settings model, persistence and
  hotkey capture logic: untouched.

### Errors / status (Phase 3)

- Copy/Save failures during an open capture session move from a modal box to
  the **status chip** (error state): non-intrusive, persists until the next
  status change, session stays open (close policy unchanged).
- Startup/system errors without a session surface (capture failure, hotkey
  conflict, unsupported OS, quick-save from tray, settings save) keep the
  native message boxes — they are system dialogs, readable and actionable.
- Native file dialogs stay native.

### Tray (Phase 2/3)

- Menu: native, labels unchanged. Icon: the embedded project icon resource
  (`assets/icon.ico` frame via `owned_app_icon`, sized `SM_CXSMICON`);
  the token-drawn `draw_icon()` remains only as a resource-missing
  fallback.

## 4. Icon set (new, original)

Simple line/vector icons drawn with GDI primitives only (`Polyline`,
`Rectangle`, `Ellipse`, `Arc`, `RoundRect`) — 18 px box, 2 px stroke, square
caps implied by GDI: `Pen`, `Line`, `Arrow`, `Rect`, `Box`, `Text`, `Highlight`,
`Mosaic`, `Solid`, `Undo`, `Redo`. No emoji, no copied artwork, no icon font,
no runtime. Copy/Save/Close stay text-labelled (their meaning is clearest as
words); they get the primary/secondary/quiet treatment instead.

## 5. Constraints checklist (must hold after implementation)

- No Electron/WPF/WebView2/new GUI crate; `windows` remains the only
  dependency.
- No `SetTimer`/`WM_TIMER`/polling/animation; no layered windows.
- One frozen frame per session; tool changes never invalidate the canvas
  (existing tests); every paint through the backbuffer, one `BitBlt`.
- Invalidations stay rect-scoped (rail/strip/status/button/selection region).
- GDI objects balanced per call (`OwnedGdi`/restore-on-drop patterns).
- Keyboard: Tab cycling, Enter activate, visible focus ring, chip tips,
  accessible names on real controls all preserved.
- `cargo check/test/clippy/build` behave at the recorded baseline or better;
  `cargo fmt --check` stays pre-existing-only (no repo-wide reformat).

## 6. QA harness note (updated)

The PowerShell QA harness `%TEMP%\opencode\ns_pb1.ps1` lives outside this
repository. For the selection-on-mouse-release task its probes were aligned
with this design: palette constants are the refreshed tokens (canvas BGRA
`16,13,12` = `#0C0D10`, surface BGRA `32,26,23` = `#171A20`), the strip probe
and the `$STRIP` click targets follow the icon-first toolbar geometry (strip
panel `510,980,670,46`, buttons at their centers), and the selection flow is
drag + mouse-up with **no Enter key** (`Drag-Region`/`Release-Region`,
`Enter-Edit` removed). The previous harness version is kept as
`ns_pb1_before_release_20261008_212919.ps1`.
