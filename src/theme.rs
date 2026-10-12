//! Single source of truth for NameShot's visual design tokens.
//!
//! Palette, type scale, spacing, radii, icon metrics and control-state colors
//! live here so no module scatters magic color values. OS-free on purpose:
//! plain constants and `const fn`s only, testable without Win32.
//!
//! Reference: `docs/UI_REFRESH_SPEC.md` §2.

/// An (r, g, b) color triple.
pub type Rgb = (u8, u8, u8);

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Main background (overlay veil blend target, general background).
pub const BG: Rgb = (0x10, 0x12, 0x16);
/// Primary surface: rail/strip panels, chip bases, Settings/About fill.
pub const SURFACE: Rgb = (0x1A, 0x1F, 0x27);
/// Elevated surface: hover faces, inputs, status chip.
pub const ELEVATED: Rgb = (0x23, 0x28, 0x33);
/// Canvas behind the fitted image.
pub const CANVAS: Rgb = (0x0C, 0x0D, 0x10);

/// Subtle 1 px hairline, shown on hover/focus/selection — rest faces stay
/// borderless (clean look).
pub const BORDER: Rgb = (0x2E, 0x34, 0x40);
/// Input/control border (edits, value box, dividers).
pub const BORDER_INPUT: Rgb = (0x3A, 0x42, 0x51);

/// Primary text (near-white).
pub const FG: Rgb = (0xF3, 0xF5, 0xF8);
/// Secondary text: hints, status chip, helper copy.
pub const FG_DIM: Rgb = (0xA4, 0xAC, 0xB9);
/// Disabled labels (WCAG-exempt; kept >= 3:1 on SURFACE).
pub const FG_DISABLED: Rgb = (0x6E, 0x75, 0x81);
/// Text/icons drawn on an accent fill.
pub const ON_ACCENT: Rgb = (0x0C, 0x12, 0x20);

/// The single vivid electric-blue accent: active tool, primary action,
/// focus rings, selection line.
pub const ACCENT: Rgb = (0x2F, 0x8F, 0xFF);
/// Accent hover (lighter tint above [`ACCENT`]).
pub const ACCENT_HOVER: Rgb = (0x5B, 0xA8, 0xFF);
/// Accent pressed.
pub const ACCENT_DOWN: Rgb = (0x1F, 0x6F, 0xD6);
/// Hover lift above [`ELEVATED`] for faces already at the elevated level.
pub const HOVER: Rgb = (0x29, 0x2F, 0x3B);
/// Quiet button pressed (sunken below SURFACE).
pub const PRESSED: Rgb = (0x12, 0x15, 0x1A);

/// Success status only.
pub const OK: Rgb = (0x4C, 0xC3, 0x8A);
/// Warning status only.
pub const WARN: Rgb = (0xE7, 0xB1, 0x4C);
/// Error status / destructive only.
pub const ERROR: Rgb = (0xE5, 0x6A, 0x6A);

/// Near-black line drawn under the selection outline for contrast over
/// arbitrary screen content.
pub const SEL_OUTLINE: Rgb = (0x05, 0x07, 0x0A);

// ---------------------------------------------------------------------------
// COLORREF packing (0x00BBGGRR) and packed aliases
// ---------------------------------------------------------------------------

/// Pack an RGB triple into a Win32 COLORREF (`0x00BBGGRR`).
pub const fn cref(c: Rgb) -> u32 {
    c.0 as u32 | ((c.1 as u32) << 8) | ((c.2 as u32) << 16)
}

/// Unpack a COLORREF (`0x00BBGGRR`) back to RGB.
pub const fn to_rgb(v: u32) -> Rgb {
    (
        (v & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        ((v >> 16) & 0xFF) as u8,
    )
}

/// Packed COLORREF of [`BG`].
pub const BG_U32: u32 = cref(BG);
/// Packed COLORREF of [`SURFACE`].
pub const SURFACE_U32: u32 = cref(SURFACE);
/// Packed COLORREF of [`ELEVATED`].
pub const ELEVATED_U32: u32 = cref(ELEVATED);
/// Packed COLORREF of [`CANVAS`].
pub const CANVAS_U32: u32 = cref(CANVAS);
/// Packed COLORREF of [`BORDER`].
pub const BORDER_U32: u32 = cref(BORDER);
/// Packed COLORREF of [`BORDER_INPUT`].
pub const BORDER_INPUT_U32: u32 = cref(BORDER_INPUT);
/// Packed COLORREF of [`FG`].
pub const FG_U32: u32 = cref(FG);
/// Packed COLORREF of [`FG_DIM`].
pub const FG_DIM_U32: u32 = cref(FG_DIM);
/// Packed COLORREF of [`FG_DISABLED`].
pub const FG_DISABLED_U32: u32 = cref(FG_DISABLED);
/// Packed COLORREF of [`ON_ACCENT`].
pub const ON_ACCENT_U32: u32 = cref(ON_ACCENT);
/// Packed COLORREF of [`ACCENT`].
pub const ACCENT_U32: u32 = cref(ACCENT);
/// Packed COLORREF of [`ACCENT_HOVER`].
pub const ACCENT_HOVER_U32: u32 = cref(ACCENT_HOVER);
/// Packed COLORREF of [`ACCENT_DOWN`].
pub const ACCENT_DOWN_U32: u32 = cref(ACCENT_DOWN);
/// Packed COLORREF of [`HOVER`].
pub const HOVER_U32: u32 = cref(HOVER);
/// Packed COLORREF of [`PRESSED`].
pub const PRESSED_U32: u32 = cref(PRESSED);
/// Packed COLORREF of [`OK`].
pub const OK_U32: u32 = cref(OK);
/// Packed COLORREF of [`WARN`].
pub const WARN_U32: u32 = cref(WARN);
/// Packed COLORREF of [`ERROR`].
pub const ERROR_U32: u32 = cref(ERROR);
/// Packed COLORREF of [`SEL_OUTLINE`].
pub const SEL_OUTLINE_U32: u32 = cref(SEL_OUTLINE);

// ---------------------------------------------------------------------------
// Control states (never communicate state by color alone — see spec §2)
// ---------------------------------------------------------------------------

/// Button/face rest fill (flush with its panel or the window surface).
pub const BTN_REST: u32 = SURFACE_U32;
/// Button hover fill.
pub const BTN_HOVER: u32 = ELEVATED_U32;
/// Button pressed / disabled fill (sunken).
pub const BTN_DOWN: u32 = PRESSED_U32;
/// Button face hairline: hover, selection and disabled faces only — rest
/// faces are borderless (clean look).
pub const BTN_BORDER: u32 = BORDER_U32;
/// Selected-tool fill (paired with a 3 px indicator bar shape cue).
pub const BTN_SELECTED: u32 = ACCENT_U32;
/// Selected-tool label/icon color.
pub const BTN_ON_SELECTED: u32 = ON_ACCENT_U32;

// ---------------------------------------------------------------------------
// Type scale
// ---------------------------------------------------------------------------

/// Font family: Segoe UI with the stock system fallback (chosen by
/// `CreateFontW` weight/height, not by this module).
pub const FONT_FAMILY: &str = "Segoe UI";
/// Control/label UI size in px.
pub const FONT_UI_PX: i32 = 16;
/// Hints, chips, helper text in px.
pub const FONT_SMALL_PX: i32 = 13;
/// Window title size in px (About only).
pub const FONT_TITLE_PX: i32 = 22;
/// Semibold weight for the title face (Win32 `FW_SEMIBOLD`).
pub const FONT_TITLE_WEIGHT: i32 = 600;

// ---------------------------------------------------------------------------
// Spacing / shape
// ---------------------------------------------------------------------------

/// Tight spacing unit.
pub const SPACE_XS: i32 = 4;
/// Base spacing unit.
pub const SPACE_S: i32 = 8;
/// Comfortable spacing unit.
pub const SPACE_M: i32 = 12;
/// Section spacing unit.
pub const SPACE_L: i32 = 16;
/// Corner radius for buttons, chips and panels (GDI `RoundRect`).
pub const RADIUS: i32 = 8;
/// Border thickness for every themed border.
pub const BORDER_W: i32 = 1;

// ---------------------------------------------------------------------------
// Icon metrics (see spec §4)
// ---------------------------------------------------------------------------

/// Icon box (square) in px.
pub const ICON_PX: i32 = 18;
/// Icon stroke width in px.
pub const ICON_STROKE: i32 = 2;
/// Selected-tool indicator bar thickness in px (shape cue).
pub const INDICATOR_W: i32 = 3;

// ---------------------------------------------------------------------------
// Chrome geometry (kept from the tested coordinate model)
// ---------------------------------------------------------------------------

/// Overlay chrome padding from the screen edge.
pub const CHROME_PAD: i32 = 8;
/// Tool rail width.
pub const RAIL_W: i32 = 60;
/// Gap between rail buttons.
pub const RAIL_GAP: i32 = 4;
/// Action strip height.
pub const STRIP_H: i32 = 46;
/// Action strip height for narrow captures (two stacked rows: colors above
/// the document actions, so the chrome stays proportionate to the shot).
pub const STRIP_H_NARROW: i32 = 72;
/// Status/size chip height.
pub const CHIP_H: i32 = 22;
/// Gap between a selection and its size chip.
pub const SELECT_CHIP_GAP: i32 = 6;

// ---------------------------------------------------------------------------
// Veil
// ---------------------------------------------------------------------------

/// Percentage of each frozen-frame pixel kept outside the selection
/// (the rest blends toward [`BG`]) for a uniform near-black veil.
pub const VEIL_KEEP_PCT: u8 = 40;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
// The assertions compare design tokens: constant by nature, still the point.
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    /// WCAG 2.x relative luminance of a linear-8 sRGB color.
    fn lum(c: Rgb) -> f64 {
        fn chan(v: u8) -> f64 {
            let s = v as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * chan(c.0) + 0.7152 * chan(c.1) + 0.0722 * chan(c.2)
    }

    /// WCAG contrast ratio between two colors (>= 1.0).
    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (l1, l2) = (lum(a), lum(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn cref_packs_red_low_byte_and_round_trips() {
        assert_eq!(cref((0x11, 0x22, 0x33)), 0x0033_2211);
        assert_eq!(to_rgb(0x0033_2211), (0x11, 0x22, 0x33));
        for c in [
            BG,
            SURFACE,
            ELEVATED,
            CANVAS,
            ACCENT,
            ACCENT_HOVER,
            HOVER,
            ERROR,
        ] {
            assert_eq!(to_rgb(cref(c)), c);
        }
    }

    #[test]
    fn colors_are_opaque_zero_alpha() {
        for v in [
            BG_U32,
            SURFACE_U32,
            ELEVATED_U32,
            CANVAS_U32,
            BORDER_U32,
            BORDER_INPUT_U32,
            FG_U32,
            FG_DIM_U32,
            FG_DISABLED_U32,
            ON_ACCENT_U32,
            ACCENT_U32,
            ACCENT_HOVER_U32,
            ACCENT_DOWN_U32,
            HOVER_U32,
            PRESSED_U32,
            OK_U32,
            WARN_U32,
            ERROR_U32,
            SEL_OUTLINE_U32,
        ] {
            assert!(v >> 24 == 0, "alpha byte must be zero: {v:#010x}");
        }
    }

    #[test]
    fn essential_text_pairs_meet_wcag_aa() {
        assert!(contrast(FG, SURFACE) >= 4.5, "FG on SURFACE");
        assert!(contrast(FG_DIM, SURFACE) >= 4.5, "FG_DIM on SURFACE");
        assert!(contrast(FG_DIM, ELEVATED) >= 4.5, "FG_DIM on ELEVATED");
        assert!(contrast(ON_ACCENT, ACCENT) >= 4.5, "ON_ACCENT on ACCENT");
        assert!(
            contrast(ON_ACCENT, ACCENT_HOVER) >= 4.5,
            "ON_ACCENT on ACCENT_HOVER"
        );
        assert!(contrast(ERROR, ELEVATED) >= 4.5, "ERROR on ELEVATED");
        assert!(contrast(ACCENT, SURFACE) >= 4.5, "ACCENT on SURFACE");
        // Disabled text is WCAG-exempt; keep it distinguishable anyway.
        assert!(contrast(FG_DISABLED, SURFACE) >= 3.0, "disabled on SURFACE");
    }

    #[test]
    fn surface_ladder_orders_rest_hover_pressed() {
        // pressed < rest < hover in luminance: sunken / flush / raised.
        assert!(lum(PRESSED) < lum(SURFACE));
        assert!(lum(SURFACE) < lum(ELEVATED));
        assert!(lum(ELEVATED) < lum(HOVER));
        assert!(lum(ACCENT_DOWN) < lum(ACCENT));
        assert!(lum(ACCENT) < lum(ACCENT_HOVER));
        assert!(lum(SEL_OUTLINE) < lum(BG));
        assert!(lum(BG) < lum(FG_DIM));
        assert!(lum(FG_DIM) < lum(FG));
    }

    #[test]
    fn state_colors_are_distinct() {
        let set = [
            BG_U32,
            SURFACE_U32,
            ELEVATED_U32,
            HOVER_U32,
            PRESSED_U32,
            ACCENT_U32,
            ACCENT_HOVER_U32,
            ACCENT_DOWN_U32,
            FG_U32,
            FG_DIM_U32,
            FG_DISABLED_U32,
            ON_ACCENT_U32,
            ERROR_U32,
            WARN_U32,
            OK_U32,
        ];
        for (i, a) in set.iter().enumerate() {
            for b in &set[i + 1..] {
                assert_ne!(a, b, "tokens must be visually distinct");
            }
        }
    }

    #[test]
    fn type_scale_and_spacing_are_ordered() {
        assert!(FONT_SMALL_PX < FONT_UI_PX);
        assert!(FONT_UI_PX < FONT_TITLE_PX);
        assert_eq!(FONT_TITLE_WEIGHT, 600);
        for s in [SPACE_XS, SPACE_S, SPACE_M, SPACE_L] {
            assert_eq!(s % SPACE_XS, 0, "spacing must be a multiple of {SPACE_XS}");
        }
        assert!(SPACE_XS < SPACE_S && SPACE_S < SPACE_M && SPACE_M < SPACE_L);
        assert!(RADIUS > 0 && BORDER_W == 1 && ICON_PX > 2 * ICON_STROKE);
        assert!(VEIL_KEEP_PCT > 0 && VEIL_KEEP_PCT < 100);
        assert!(RAIL_GAP > 0 && SELECT_CHIP_GAP > 0 && CHIP_H >= FONT_SMALL_PX);
    }
}
