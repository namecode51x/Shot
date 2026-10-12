//! Pure, testable placement of the editing chrome (tool rail, action strip,
//! status chip) around the selection.
//!
//! [`plan`] measures the required sizes, then searches in two phases: first
//! only usable-size chrome (Expanded/Compact button heights) placed fully
//! clear of the selection, ranked by minimum movement from the ideal
//! anchors; only when nothing clear fits does it fall back to the floor
//! metrics, edge pins and covering placements (EdgeFallback). Every
//! candidate that leaves the available work area or whose rectangles
//! intersect is rejected. The work area is the selected
//! monitor's `rcWork` (client-clipped), so chrome never spills onto the
//! taskbar; the selection itself may legitimately extend outside it. No
//! windows, no state — inputs are plain rectangles and sizes, so every rule
//! is unit-testable.

/// Plain rectangle in client coordinates. The OS-free twin of the Win32
/// `RECT`; the Windows boundary converts with `From` inside `overlay::edit`,
/// so this file stays free of OS imports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Which placement strategy produced a [`Layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Full-size chrome beside a selection that has room on all sides.
    Expanded,
    /// Metrics shrank or panels moved, but nothing is pinned to an edge and
    /// nothing covers the selection.
    Compact,
    /// No room left: a panel is pinned to the viewport edge or sits on top
    /// of the selection.
    EdgeFallback,
}

/// Everything [`plan`] needs to know about the current client and chrome.
pub struct Measures {
    /// Selection rect in client coordinates.
    pub canvas: Rect,
    /// Client size `(width, height)`; the client origin is `(0, 0)`.
    pub client: (i32, i32),
    /// Available work area in client coordinates (the selected monitor's
    /// `rcWork`, translated from screen space): the region not covered by
    /// the taskbar or other app-reserved bands. Chrome must stay inside it;
    /// the selection may extend beyond it. Clipped to the client before use.
    pub work: Rect,
    pub rail_w: i32,
    /// Number of buttons stacked in the rail.
    pub rail_items: i32,
    pub rail_gap: i32,
    /// Preferred / minimum / absolute-floor rail button height.
    pub th_max: i32,
    pub th_min: i32,
    pub th_floor: i32,
    /// Ideal (unscaled) strip content width.
    pub strip_ideal: i32,
    pub strip_h: i32,
    pub chip_w: i32,
    pub chip_h: i32,
    pub pad: i32,
    /// Gap between the selection and the strip.
    pub chip_gap: i32,
    /// Minimum distance from any viewport edge.
    pub margin: i32,
}

/// The chosen chrome placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub mode: Mode,
    pub rail: Rect,
    pub strip: Rect,
    /// Status/tooltip chip; empty (`right <= left`) when nothing fits.
    pub chip: Rect,
    /// Rail button height actually used.
    pub th: i32,
    /// Rail button gap actually used (may shrink to `0` at the floor).
    pub rail_gap: i32,
}

#[derive(Clone, Copy)]
struct Profile {
    rank: u8,
    gap0: i32,
    gap1: i32,
    gap_n: usize,
    th_min: i32,
    pins: bool,
}

struct Best {
    score: (u8, i32, u8, i32),
    rail: Rect,
    strip: Rect,
    chip: Rect,
    th: i32,
    gap: i32,
    pins: bool,
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// The effective placement bounds: `work` (client coords) clipped to the
/// client. Falls back to the whole client when `work` is empty or does not
/// intersect it, so a bogus work rect can never strand the chrome.
fn bounds_of(m: &Measures) -> Rect {
    let (cw, ch) = m.client;
    let w = &m.work;
    let r = Rect {
        left: w.left.max(0),
        top: w.top.max(0),
        right: w.right.min(cw),
        bottom: w.bottom.min(ch),
    };
    if r.right > r.left && r.bottom > r.top {
        r
    } else {
        Rect {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        }
    }
}

fn in_bounds(r: &Rect, b: &Rect) -> bool {
    r.right > r.left
        && r.bottom > r.top
        && r.left >= b.left
        && r.top >= b.top
        && r.right <= b.right
        && r.bottom <= b.bottom
}

fn clamp_lo(v: i32, lo: i32, hi: i32) -> i32 {
    let hi = hi.max(lo);
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn find_chip(m: &Measures, rail: &Rect, strip: &Rect, b: &Rect) -> Option<Rect> {
    let chip_above_first = strip.top >= m.canvas.bottom;
    for primary_above in [chip_above_first, !chip_above_first] {
        let band: Option<(i32, i32)> = if primary_above {
            let top = strip.top - 4 - m.chip_h;
            (top >= b.top).then_some((top, strip.top - 4))
        } else {
            let top = strip.bottom + 4;
            (top + m.chip_h <= b.bottom).then_some((top, top + m.chip_h))
        };
        let Some((top, bottom)) = band else { continue };
        let xs = [
            strip.left,
            rail.right + m.pad,
            strip.right - m.chip_w,
            b.right - m.chip_w - m.pad,
        ];
        for x in xs {
            let chip = Rect {
                left: x,
                top,
                right: x + m.chip_w,
                bottom,
            };
            if in_bounds(&chip, b) && !overlap(rail, &chip) && !overlap(strip, &chip) {
                return Some(chip);
            }
        }
    }
    None
}

/// Place the rail/strip/chip for `m`.
///
/// Guarantees: every returned non-empty rect lies inside the available work
/// area (the monitor's `rcWork`, clipped to the client), the rail/strip/chip
/// are pairwise non-overlapping, and a non-empty chip docks at
/// `strip.top - 4` (above) or `strip.bottom + 4` (below).
///
/// Two phases: first only usable-size buttons (`th >= th_min`, the Compact
/// minimum whose 18 px icons still fit) that never cover the selection are
/// accepted, ranked by minimum movement from the ideal anchors. Only when no
/// such placement exists does the search fall back to the floor metrics,
/// edge pins and covering placements (EdgeFallback). Without the first
/// phase, tiny selections (e.g. 50x50) collapse the rail to the 14 px floor
/// — crushed icons — just to shave movement, even though a clear placement
/// with usable buttons exists.
pub fn plan(m: &Measures) -> Layout {
    let wb = bounds_of(m);
    let n = m.rail_items.max(1);
    let rail_h = |th: i32, gap: i32| th * n + gap * (n - 1);

    let c = &m.canvas;
    let fits_left = c.left - m.pad - m.rail_w >= wb.left + m.margin;
    let fits_right = c.right + m.pad + m.rail_w <= wb.right - m.margin;
    let ideal_rail_x = if fits_left {
        c.left - m.pad - m.rail_w
    } else if fits_right {
        c.right + m.pad
    } else {
        (c.left - m.pad - m.rail_w).max(wb.left + m.margin)
    };

    let strip_fits_w = m.strip_ideal + 2 * m.pad <= wb.right - wb.left;
    let strip_w = if strip_fits_w {
        m.strip_ideal + 2 * m.pad
    } else {
        wb.right - wb.left
    };
    let centered_x = if strip_fits_w {
        clamp_lo(
            (c.left + c.right) / 2 - strip_w / 2,
            wb.left,
            wb.right - strip_w,
        )
    } else {
        wb.left
    };
    let below_fits = c.bottom + m.chip_gap + m.strip_h <= wb.bottom - m.pad;
    let below_y = c.bottom + m.chip_gap;
    let above_y = c.top - m.chip_gap - m.strip_h;
    let above_fits = above_y >= wb.top + m.pad;
    let ideal_strip_y = if below_fits {
        below_y
    } else if above_fits {
        above_y
    } else {
        clamp_lo(
            wb.bottom - m.strip_h - m.pad,
            wb.top + m.pad,
            wb.bottom - m.strip_h,
        )
    };

    let mut sides: Vec<i32> = Vec::new();
    if fits_left {
        sides.push(c.left - m.pad - m.rail_w);
    }
    if fits_right {
        sides.push(c.right + m.pad);
    }
    sides.push((c.left - m.pad - m.rail_w).max(wb.left + m.margin));
    sides.dedup();

    let gap_rail = m.rail_gap;
    let profiles = [
        Profile {
            rank: 0,
            gap0: gap_rail,
            gap1: 0,
            gap_n: 1,
            th_min: m.th_max,
            pins: false,
        },
        Profile {
            rank: 1,
            gap0: gap_rail,
            gap1: 0,
            gap_n: 1,
            th_min: m.th_min,
            pins: false,
        },
        Profile {
            rank: 2,
            gap0: gap_rail,
            gap1: 0,
            gap_n: 2,
            th_min: m.th_floor,
            pins: true,
        },
    ];

    let ctx = Ctx {
        m,
        wb: &wb,
        sides: &sides,
        strip_fits_w,
        strip_w,
        centered_x,
        ideal_rail_x,
        ideal_strip_y,
        below_fits,
        below_y,
        above_fits,
        above_y,
        rail_h: &rail_h,
    };
    // Usable buttons clear of the selection first; the floor/pins/covering
    // fallback only runs when nothing clear fits.
    if let Some(b) = search(&ctx, &profiles[..2], true) {
        return finish(m, b);
    }
    if let Some(b) = search(&ctx, &profiles, false) {
        return finish(m, b);
    }
    emergency(m)
}

/// Shared inputs for [`search`]: precomputed geometry plus the ideal anchors
/// the movement score is measured against.
struct Ctx<'a> {
    m: &'a Measures,
    wb: &'a Rect,
    sides: &'a [i32],
    strip_fits_w: bool,
    strip_w: i32,
    centered_x: i32,
    ideal_rail_x: i32,
    ideal_strip_y: i32,
    below_fits: bool,
    below_y: i32,
    above_fits: bool,
    above_y: i32,
    rail_h: &'a dyn Fn(i32, i32) -> i32,
}

/// Cheapest candidate over `profiles`. When `require_clear` is set, any
/// candidate touching the selection is rejected, so the caller only ever
/// sees usable-size chrome placed fully clear of it.
fn search(ctx: &Ctx<'_>, profiles: &[Profile], require_clear: bool) -> Option<Best> {
    let m = ctx.m;
    let wb = ctx.wb;
    let c = &m.canvas;
    let mut best: Option<Best> = None;

    for profile in profiles {
        let gaps = [profile.gap0, profile.gap1];
        let th_bot = profile.th_min.min(m.th_max);
        for th in (th_bot..=m.th_max).rev() {
            for &gap in gaps.iter().take(profile.gap_n) {
                let h = (ctx.rail_h)(th, gap);
                if h <= 0 {
                    continue;
                }
                let lo = wb.top + m.margin;
                let hi = (wb.bottom - h - m.margin).max(lo);
                let ideal_rail_y = clamp_lo(c.top, lo, hi);
                let mut anchors = vec![ideal_rail_y, clamp_lo(c.bottom - h, lo, hi)];
                if profile.pins {
                    anchors.push(lo);
                }
                anchors.dedup();
                for &rail_x in ctx.sides {
                    for &rail_y in &anchors {
                        let rail = Rect {
                            left: rail_x,
                            top: rail_y,
                            right: rail_x + m.rail_w,
                            bottom: rail_y + h,
                        };
                        if !in_bounds(&rail, wb) {
                            continue;
                        }
                        let mut ys: Vec<i32> = Vec::new();
                        if ctx.below_fits {
                            ys.push(ctx.below_y);
                        }
                        if ctx.above_fits {
                            ys.push(ctx.above_y);
                        }
                        if profile.pins {
                            ys.push(clamp_lo(
                                wb.bottom - m.strip_h - m.pad,
                                wb.top + m.pad,
                                wb.bottom - m.strip_h,
                            ));
                            ys.push(wb.top + m.pad);
                        }
                        ys.dedup();
                        for strip_y in ys {
                            let mut xs: Vec<i32> = vec![ctx.centered_x];
                            if ctx.strip_fits_w {
                                xs.push(rail.right + m.pad);
                                xs.push(rail.left - m.pad - ctx.strip_w);
                            }
                            xs.dedup();
                            for strip_x in xs {
                                if strip_x < wb.left || strip_x + ctx.strip_w > wb.right {
                                    continue;
                                }
                                let strip = Rect {
                                    left: strip_x,
                                    top: strip_y,
                                    right: strip_x + ctx.strip_w,
                                    bottom: strip_y + m.strip_h,
                                };
                                if !in_bounds(&strip, wb) || overlap(&rail, &strip) {
                                    continue;
                                }
                                let Some(chip) = find_chip(m, &rail, &strip, wb) else {
                                    continue;
                                };
                                let covers =
                                    (overlap(&rail, c) || overlap(&strip, c) || overlap(&chip, c))
                                        as u8;
                                if require_clear && covers > 0 {
                                    continue;
                                }
                                let movement = (rail.left - ctx.ideal_rail_x).abs()
                                    + (rail.top - ideal_rail_y).abs()
                                    + (strip.left - ctx.centered_x).abs()
                                    + (strip.top - ctx.ideal_strip_y).abs();
                                let score = (covers, movement, profile.rank, -th);
                                let better = match &best {
                                    None => true,
                                    Some(b) => score < b.score,
                                };
                                if better {
                                    best = Some(Best {
                                        score,
                                        rail,
                                        strip,
                                        chip,
                                        th,
                                        gap,
                                        pins: profile.pins,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    best
}

/// Score a [`Best`] into its public [`Layout`].
fn finish(m: &Measures, b: Best) -> Layout {
    let (covers, _movement, rank, _) = b.score;
    let mode = if covers > 0 || b.pins {
        Mode::EdgeFallback
    } else if rank == 0 && b.th == m.th_max && b.gap == m.rail_gap {
        Mode::Expanded
    } else {
        Mode::Compact
    };
    Layout {
        mode,
        rail: b.rail,
        strip: b.strip,
        chip: b.chip,
        th: b.th,
        rail_gap: b.gap,
    }
}

/// Degenerate-bounds safety net (no candidate fits): clamped, in-work-area
/// rects with an empty chip. Never hit by realistic overlay sizes.
fn emergency(m: &Measures) -> Layout {
    let wb = bounds_of(m);
    let strip_y = clamp_lo(
        wb.bottom - m.strip_h - m.pad,
        wb.top,
        (wb.bottom - m.strip_h).max(wb.top),
    );
    let strip = Rect {
        left: wb.left,
        top: strip_y,
        right: wb.right,
        bottom: strip_y + m.strip_h,
    };
    let avail = (strip_y - wb.top - 2 * m.margin).max(0);
    let n = m.rail_items.max(1);
    let th = clamp_lo(avail / n, 4, m.th_max);
    let h = th * n;
    let rail = Rect {
        left: wb.left + m.margin,
        top: wb.top + m.margin,
        right: wb.left + m.margin + m.rail_w,
        bottom: wb.top + m.margin + h.min(avail),
    };
    Layout {
        mode: Mode::EdgeFallback,
        rail,
        strip,
        chip: Rect::default(),
        th,
        rail_gap: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CW: i32 = 1920;
    const CH: i32 = 1200;

    fn measures(canvas: Rect, client: (i32, i32)) -> Measures {
        Measures {
            canvas,
            client,
            work: Rect {
                left: 0,
                top: 0,
                right: client.0,
                bottom: client.1,
            },
            rail_w: 60,
            rail_items: 9,
            rail_gap: 4,
            th_max: 38,
            th_min: 20,
            th_floor: 14,
            strip_ideal: 670,
            strip_h: 46,
            chip_w: 340,
            chip_h: 22,
            pad: 8,
            chip_gap: 30,
            margin: 4,
        }
    }

    fn rect(l: i32, t: i32, r: i32, b: i32) -> Rect {
        Rect {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    fn assert_sound(l: &Layout, m: &Measures) {
        let wb = bounds_of(m);
        assert!(
            in_bounds(&l.rail, &wb),
            "rail out of the work area: {:?}",
            l.rail
        );
        assert!(
            in_bounds(&l.strip, &wb),
            "strip out of the work area: {:?}",
            l.strip
        );
        if l.chip.right > l.chip.left {
            assert!(
                in_bounds(&l.chip, &wb),
                "chip out of the work area: {:?}",
                l.chip
            );
            assert!(!overlap(&l.rail, &l.chip), "rail overlaps chip: {l:?}");
            assert!(!overlap(&l.strip, &l.chip), "strip overlaps chip: {l:?}");
            assert!(
                l.chip.bottom == l.strip.top - 4 || l.chip.top == l.strip.bottom + 4,
                "chip not docked to the strip: {l:?}"
            );
        }
        assert!(!overlap(&l.rail, &l.strip), "rail overlaps strip: {l:?}");
        assert!(l.th >= 14, "button height collapsed: {}", l.th);
    }

    #[test]
    fn reported_330x207_case_never_crosses_and_stays_off_the_selection() {
        let m = measures(rect(1570, 910, 1900, 1117), (CW, CH));
        let l = plan(&m);
        assert_sound(&l, &m);
        assert!(!overlap(&l.rail, &m.canvas), "rail on the selection: {l:?}");
        assert!(
            !overlap(&l.strip, &m.canvas),
            "strip on the selection: {l:?}"
        );
        assert!(!overlap(&l.chip, &m.canvas), "chip on the selection: {l:?}");
        assert_eq!(
            l.mode,
            Mode::Compact,
            "rail shrinks to the selection height: {l:?}"
        );
        assert!(
            l.rail.right <= m.canvas.left,
            "rail beside the left edge: {l:?}"
        );
        assert!((20..38).contains(&l.th), "compact button height: {}", l.th);
    }

    #[test]
    fn roomy_selection_keeps_the_full_size_expanded_layout() {
        let m = measures(rect(200, 250, 1500, 950), (CW, CH));
        let l = plan(&m);
        assert_sound(&l, &m);
        assert_eq!(l.mode, Mode::Expanded, "{l:?}");
        assert_eq!(l.th, 38, "{l:?}");
        assert_eq!(l.rail.left, 200 - 8 - 60, "{l:?}");
        assert_eq!(l.rail.right, 200 - 8, "{l:?}");
        assert_eq!(l.strip.top, 950 + 30, "{l:?}");
    }

    #[test]
    fn tiny_and_huge_selections_stay_disjoint_and_in_bounds() {
        for canvas in [
            rect(0, 0, 40, 30),
            rect(CW - 40, CH - 30, CW, CH),
            rect(0, 0, CW, CH),
            rect(10, 10, CW - 10, CH - 10),
        ] {
            let m = measures(canvas, (CW, CH));
            let l = plan(&m);
            assert_sound(&l, &m);
        }
    }

    #[test]
    fn tiny_selection_keeps_usable_buttons_clear_of_the_canvas() {
        // Reported small-shot case (50x50): the rail used to collapse to
        // the 14 px floor — crushed 18 px icons — just to shave movement,
        // even though a clear placement with usable buttons exists. Phase
        // one of `plan` must keep `th` at the Compact minimum (20) or
        // better and keep every panel off the tiny canvas.
        for canvas in [
            rect(935, 575, 985, 625),
            rect(100, 100, 150, 150),
            rect(600, 150, 650, 200),
            rect(1570, 910, 1620, 960),
        ] {
            let m = measures(canvas, (CW, CH));
            let l = plan(&m);
            assert_sound(&l, &m);
            assert!(!overlap(&l.rail, &m.canvas), "rail on tiny canvas: {l:?}");
            assert!(
                !overlap(&l.strip, &m.canvas),
                "strip on tiny canvas: {l:?}"
            );
            assert!(!overlap(&l.chip, &m.canvas), "chip on tiny canvas: {l:?}");
            assert!(
                l.th >= 20,
                "rail buttons crushed below the usable minimum: {}",
                l.th
            );
            assert_ne!(l.mode, Mode::EdgeFallback, "tiny canvas fell back: {l:?}");
        }
    }

    #[test]
    fn narrow_strip_config_stays_proportionate_on_small_captures() {
        // Two-row strip (398x72) used by narrow captures: same guarantees
        // as the full strip — clear of the canvas, usable buttons, sound
        // chrome — at a width that no longer dwarfs the shot.
        for canvas in [
            rect(15, 110, 212, 258), // reported 197x148 case
            rect(860, 500, 1057, 648),
            rect(935, 575, 985, 625), // 50x50
        ] {
            let mut m = measures(canvas, (CW, CH));
            m.strip_ideal = 398;
            m.strip_h = 72;
            let l = plan(&m);
            assert_sound(&l, &m);
            assert!(!overlap(&l.rail, &m.canvas), "rail on tiny canvas: {l:?}");
            assert!(
                !overlap(&l.strip, &m.canvas),
                "strip on tiny canvas: {l:?}"
            );
            assert!(!overlap(&l.chip, &m.canvas), "chip on tiny canvas: {l:?}");
            assert!(
                l.th >= 20,
                "rail buttons crushed below the usable minimum: {}",
                l.th
            );
            assert_eq!(
                l.strip.bottom - l.strip.top,
                72,
                "narrow strip height must survive planning: {l:?}"
            );
        }
    }

    #[test]
    fn every_corner_and_every_client_size_stays_sound() {
        for (cw, ch) in [(1920, 1200), (1024, 768), (800, 600)] {
            for canvas in [
                rect(0, 0, 330, 207),
                rect(cw - 330, 0, cw, 207),
                rect(0, ch - 207, 330, ch),
                rect(cw - 330, ch - 207, cw, ch),
                rect(cw / 2 - 165, ch / 2 - 103, cw / 2 + 165, ch / 2 + 103),
            ] {
                let m = measures(canvas, (cw, ch));
                let l = plan(&m);
                assert_sound(&l, &m);
            }
        }
    }

    #[test]
    fn selection_filling_the_client_uses_the_edge_fallback() {
        let m = measures(rect(0, 0, CW, CH), (CW, CH));
        let l = plan(&m);
        assert_sound(&l, &m);
        assert_eq!(l.mode, Mode::EdgeFallback, "{l:?}");
    }

    #[test]
    fn neither_rail_nor_strip_fits_uses_the_edge_fallback() {
        let m = measures(rect(10, 10, 690, 390), (700, 400));
        let l = plan(&m);
        assert_sound(&l, &m);
        assert_eq!(l.mode, Mode::EdgeFallback, "{l:?}");
        assert!(
            l.strip.bottom <= 400 - 8,
            "strip pinned to the bottom: {l:?}"
        );
        assert!(l.chip.right > l.chip.left, "chip still reachable: {l:?}");
    }

    #[test]
    fn selection_touching_both_bands_pins_the_strip_without_crossing() {
        let canvas = rect(600, 40, 930, 1160);
        let m = measures(canvas, (CW, CH));
        let l = plan(&m);
        assert_sound(&l, &m);
        assert_eq!(
            l.mode,
            Mode::EdgeFallback,
            "no band fits below/above: {l:?}"
        );
        assert!(!overlap(&l.strip, &canvas), "strip on the selection: {l:?}");
        assert!(!overlap(&l.rail, &l.strip), "{l:?}");
    }

    #[test]
    fn negative_and_degenerate_canvas_inputs_do_not_panic() {
        for canvas in [
            rect(-10, -10, 320, 197),
            rect(-500, -500, -400, -400),
            rect(100, 100, 100, 100),
            rect(CW - 1, CH - 1, CW + 400, CH + 400),
        ] {
            let m = measures(canvas, (CW, CH));
            let l = plan(&m);
            let wb = bounds_of(&m);
            assert!(in_bounds(&l.rail, &wb), "rail escaped: {:?}", l.rail);
            assert!(in_bounds(&l.strip, &wb), "strip escaped: {:?}", l.strip);
        }
    }

    #[test]
    fn bottom_taskbar_work_area_keeps_all_chrome_above_it() {
        // The monitor rect is 1920x1200; the taskbar owns y 1160..1200, so
        // the available work area bottoms out at 1160. The reported low
        // selections (A: reported 330x207 case, D: bottom-right edge) used
        // to let the rail spill onto the taskbar band.
        let work = rect(0, 0, CW, 1160);
        for canvas in [
            rect(1570, 910, 1900, 1117), // scenario A
            rect(1640, 990, 1900, 1150), // scenario D (low)
            rect(0, 1000, 400, 1199),    // selection hanging into the taskbar
        ] {
            let mut m = measures(canvas, (CW, CH));
            m.work = work;
            let l = plan(&m);
            assert_sound(&l, &m);
            assert!(l.rail.bottom <= 1160, "rail crossed the taskbar: {l:?}");
            assert!(l.strip.bottom <= 1160, "strip crossed the taskbar: {l:?}");
            if l.chip.right > l.chip.left {
                assert!(l.chip.bottom <= 1160, "chip crossed the taskbar: {l:?}");
            }
        }
    }

    #[test]
    fn work_area_offset_to_a_secondary_monitor_stays_confined() {
        // Two 1920x1200 monitors side by side; client space spans both
        // (3840x1200), the right monitor's work area excludes the taskbar
        // band of the primary (which sits on its bottom edge).
        let mut m = measures(rect(3400, 900, 3700, 1100), (3840, 1200));
        m.work = rect(1920, 0, 3840, 1160);
        let l = plan(&m);
        assert_sound(&l, &m);
        assert!(l.rail.left >= 1920, "rail left the right monitor: {l:?}");
        assert!(l.rail.bottom <= 1160, "rail crossed the taskbar: {l:?}");
        assert!(l.strip.bottom <= 1160, "strip crossed the taskbar: {l:?}");
    }

    #[test]
    fn empty_or_disjoint_work_rect_degrades_to_the_client() {
        // A bogus work area (e.g. a failed GetMonitorInfo fallback) must not
        // strand the chrome; bounds fall back to the full client.
        for work in [rect(0, 0, 0, 0), rect(5000, 5000, 6000, 6000)] {
            let mut m = measures(rect(200, 250, 1500, 950), (CW, CH));
            m.work = work;
            let l = plan(&m);
            assert_sound(&l, &m);
            assert_eq!(l.mode, Mode::Expanded, "{l:?}");
        }
    }

    #[test]
    fn chip_docks_away_from_the_selection_on_both_bands() {
        let below_m = measures(rect(200, 250, 1500, 950), (CW, CH));
        let below = plan(&below_m);
        assert_eq!(
            below.chip.bottom,
            below.strip.top - 4,
            "chip above the strip"
        );
        let above_m = measures(rect(300, 400, 700, 1160), (CW, CH));
        let above = plan(&above_m);
        assert_eq!(
            above.chip.top,
            above.strip.bottom + 4,
            "chip below the strip"
        );
        assert_sound(&above, &above_m);
    }
}
