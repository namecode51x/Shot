//! Ordered rasterization of annotations into a fresh raster image.
//!
//! The only value that ever reaches the clipboard or a file is the output of
//! [`flatten`]: a new `Frame` built by copying the source and applying every
//! annotation **in list order**. Source pixels exist only inside `source`;
//! there are no layers, no side channels, no metadata.

use crate::annotate::Shape;
use crate::image::{Frame, IRect, redact};

/// Apply one annotation to `dst`.
pub fn apply(dst: &mut Frame, shape: &Shape) {
    match shape {
        Shape::Pen { pts, color, width } => dst.polyline(pts, *color, *width),
        Shape::Line { a, b, color, width } => dst.line(a.0, a.1, b.0, b.1, *color, *width),
        Shape::Arrow { a, b, color, width } => dst.arrow(a.0, a.1, b.0, b.1, *color, *width),
        Shape::Rect {
            rect,
            color,
            width,
            filled,
        } => {
            if *filled {
                dst.fill_rect(*rect, *color)
            } else {
                dst.stroke_rect(*rect, *color, *width)
            }
        }
        Shape::Text {
            at,
            text,
            color,
            size,
        } => crate::platform::text::draw_text(dst, *at, text, *color, *size),
        Shape::Highlight { a, b, color, width } => dst.line(a.0, a.1, b.0, b.1, *color, *width),
        Shape::Mosaic { rect, block } => redact::pixelate(dst, *rect, *block),
        Shape::Solid { rect, color } => redact::solid(dst, *rect, *color),
    }
}

/// Build the final image: `source` + all annotations in order.
/// The source frame is not modified.
pub fn flatten(source: &Frame, items: &[Shape]) -> Frame {
    let mut out = source.clone();
    for s in items {
        apply(&mut out, s);
    }
    out
}

/// Union of two rects (empty-aware).
fn union_rect(a: &IRect, b: &IRect) -> IRect {
    if a.is_empty() {
        return *b;
    }
    if b.is_empty() {
        return *a;
    }
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    IRect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

/// Copy pixel rows `[r.y, r.bottom())` from `src` to `dst` (same dims).
/// Full-copy fallback when the dims ever disagree (defensive: every
/// construction path keeps the three buffers identical).
fn copy_rows(dst: &mut Frame, src: &Frame, r: &IRect) {
    if r.is_empty() {
        return;
    }
    if dst.w != src.w || dst.h != src.h {
        dst.px.copy_from_slice(&src.px);
        return;
    }
    let stride = dst.w as usize * 4;
    for y in r.y..r.bottom() {
        let s = (y as usize * stride) + r.x as usize * 4;
        let e = s + r.w as usize * 4;
        dst.px[s..e].copy_from_slice(&src.px[s..e]);
    }
}

/// Incremental renderer for the editor.
///
/// Keeps the flattened state of `items[..committed_len]` and only applies the
/// delta on each call (annotation history only ever changes at the tail, which
/// [`crate::annotate::History`] guarantees). An in-progress (draft) annotation
/// is rendered into a separate view buffer so it never contaminates the
/// committed state — but only the rows it (and the previous draft position)
/// can touch are refreshed, so a mousemove repaints O(draft) rows instead of
/// the whole frame.
pub struct Preview {
    source: Frame,
    committed: Frame,
    view: Frame,
    committed_len: usize,
    /// Rows of `view` holding the last rendered draft (already padded and
    /// frame-clamped). None when the view carries no draft pixels.
    last_draft: Option<IRect>,
}

impl Preview {
    pub fn new(source: Frame) -> Preview {
        let committed = source.clone();
        let view = source.clone();
        Preview {
            source,
            committed,
            view,
            committed_len: 0,
            last_draft: None,
        }
    }

    pub fn source(&self) -> &Frame {
        &self.source
    }

    pub fn committed(&self) -> &Frame {
        &self.committed
    }

    pub fn reset(&mut self) {
        self.committed = self.source.clone();
        self.view = self.source.clone();
        self.committed_len = 0;
        self.last_draft = None;
    }

    pub fn render(&mut self, items: &[Shape], draft: Option<&Shape>) -> &Frame {
        let mut committed_touched = self.committed_len > items.len();
        if committed_touched {
            // Tail was removed (undo) — the cached prefix may no longer match.
            self.committed.px.copy_from_slice(&self.source.px);
            self.committed_len = 0;
        }
        for i in self.committed_len..items.len() {
            apply(&mut self.committed, &items[i]);
            committed_touched = true;
        }
        self.committed_len = items.len();

        match draft {
            Some(d) => {
                let frame = self.view.bounds();
                // Text drafts estimate their raster extent: refresh the whole
                // view rather than trusting the estimate. (Live text edits use
                // a native EDIT control, so this path only fires in tests and
                // exotic callers — correctness first.)
                let full_refresh =
                    committed_touched || matches!(d, Shape::Text { .. });
                if full_refresh {
                    self.view.px.copy_from_slice(&self.committed.px);
                    // Whole view is dirty until proven otherwise.
                    self.last_draft = Some(frame);
                } else {
                    let painted = d.bounds().expand(2).intersect(&frame);
                    let dirty = match self.last_draft {
                        Some(prev) => union_rect(&prev, &painted).intersect(&frame),
                        None => painted,
                    };
                    copy_rows(&mut self.view, &self.committed, &dirty);
                    self.last_draft = Some(painted);
                }
                apply(&mut self.view, d);
                &self.view
            }
            None => {
                // Committed pixels moved under the view: the whole view is
                // dirty until a draft (or paint without one) proves otherwise.
                // Otherwise restore the last draft rows so the invariant
                // holds: view == committed whenever there is no draft.
                // Steady state (no draft, untouched, clean) costs nothing.
                if committed_touched {
                    self.last_draft = Some(self.view.bounds());
                } else if let Some(prev) = self.last_draft {
                    copy_rows(&mut self.view, &self.committed, &prev);
                    self.last_draft = None;
                }
                &self.committed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{IRect, Rgba};

    fn checker(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h).unwrap();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let v = if (x / 4 + y / 4) % 2 == 0 { 30 } else { 220 };
                let i = f.index(x, y).unwrap();
                f.px[i] = v;
                f.px[i + 1] = v;
                f.px[i + 2] = v;
                f.px[i + 3] = 255;
            }
        }
        f
    }

    #[test]
    fn flatten_does_not_modify_source() {
        let src = checker(16, 16);
        let snapshot = src.clone();
        let items = vec![Shape::Rect {
            rect: IRect::new(2, 2, 6, 6),
            color: Rgba::opaque(255, 0, 0),
            width: 1,
            filled: true,
        }];
        let out = flatten(&src, &items);
        assert_eq!(src, snapshot);
        assert_eq!(out.get(3, 3).unwrap(), [0, 0, 255, 255]);
        assert!(out.diff_pixels(&src) >= 36);
    }

    #[test]
    fn redaction_applies_on_top_of_earlier_shapes() {
        let src = checker(20, 20);
        let items = vec![
            Shape::Rect {
                rect: IRect::new(4, 4, 8, 8),
                color: Rgba::opaque(255, 0, 0),
                width: 2,
                filled: true,
            },
            Shape::Mosaic {
                rect: IRect::new(4, 4, 8, 8),
                block: 4,
            },
        ];
        let out = flatten(&src, &items);
        // Order matters: the mosaic is applied to the state produced by the
        // rectangle, so every region pixel is the mean of the *red* pixels the
        // shape painted — the original checker pattern is gone from the output.
        for y in 4..12 {
            for x in 4..12 {
                assert_eq!(out.get(x, y).unwrap(), [0, 0, 255, 255], "pixel {x},{y}");
            }
        }
        assert!(out.diff_pixels(&src) >= 64);
        // Sanity: the mosaic of the untouched source would NOT be pure red.
        let direct = flatten(&src, &items[1..]);
        assert_ne!(direct.get(5, 5).unwrap(), [0, 0, 255, 255]);
    }

    #[test]
    fn mosaic_without_prior_shapes_equals_source_block_means() {
        let src = checker(20, 20);
        let items = vec![Shape::Mosaic {
            rect: IRect::new(0, 0, 8, 8),
            block: 4,
        }];
        let out = flatten(&src, &items);
        for y in 0..8 {
            for x in 0..8 {
                // Expected mean comes from the block that contains (x, y).
                let bx = (x / 4) * 4;
                let by = (y / 4) * 4;
                let mut sum = 0u32;
                for yy in by..by + 4 {
                    for xx in bx..bx + 4 {
                        sum += src.get(xx, yy).unwrap()[0] as u32;
                    }
                }
                let mean = ((sum + 8) / 16) as u8;
                assert_eq!(out.get(x, y).unwrap()[0], mean, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn solid_redaction_after_text_like_shape_is_opaque() {
        let src = checker(10, 10);
        let items = vec![
            Shape::Rect {
                rect: IRect::new(0, 0, 10, 10),
                color: Rgba::opaque(0, 255, 0),
                width: 1,
                filled: true,
            },
            Shape::Solid {
                rect: IRect::new(1, 1, 4, 4),
                color: Rgba::new(9, 9, 9, 1),
            },
        ];
        let out = flatten(&src, &items);
        for y in 1..5 {
            for x in 1..5 {
                assert_eq!(out.get(x, y).unwrap(), [9, 9, 9, 255]);
            }
        }
        assert_eq!(out.get(6, 6).unwrap(), [0, 255, 0, 255]);
    }

    #[test]
    fn preview_incremental_matches_full_flatten() {
        let src = checker(24, 24);
        let mut preview = Preview::new(src.clone());
        let mut items: Vec<Shape> = Vec::new();
        items.push(Shape::Line {
            a: (0, 0),
            b: (23, 23),
            color: Rgba::opaque(255, 255, 0),
            width: 3,
        });
        let r1 = preview.render(&items, None).clone();
        assert_eq!(r1, flatten(&src, &items));

        items.push(Shape::Mosaic {
            rect: IRect::new(2, 2, 10, 10),
            block: 4,
        });
        let r2 = preview.render(&items, None).clone();
        assert_eq!(r2, flatten(&src, &items));

        // undo back below the cached prefix, then a different push
        items.pop();
        let r3 = preview.render(&items, None).clone();
        assert_eq!(r3, flatten(&src, &items));
        items.push(Shape::Solid {
            rect: IRect::new(0, 0, 5, 5),
            color: Rgba::opaque(1, 2, 3),
        });
        let r4 = preview.render(&items, None).clone();
        assert_eq!(r4, flatten(&src, &items));
    }

    #[test]
    fn preview_draft_is_not_committed() {
        let src = checker(8, 8);
        let mut preview = Preview::new(src.clone());
        let items: Vec<Shape> = vec![Shape::Line {
            a: (0, 0),
            b: (7, 0),
            color: Rgba::opaque(255, 0, 0),
            width: 1,
        }];
        let draft = Shape::Line {
            a: (0, 7),
            b: (7, 7),
            color: Rgba::opaque(0, 0, 255),
            width: 1,
        };
        let with_draft = preview.render(&items, Some(&draft)).clone();
        assert_ne!(with_draft, flatten(&src, &items));
        assert_eq!(
            with_draft,
            flatten(&src, &[items.clone(), vec![draft.clone()]].concat())
        );
        let without = preview.render(&items, None).clone();
        assert_eq!(without, flatten(&src, &items));
    }

    #[test]
    fn preview_reset_restores_source() {
        let src = checker(8, 8);
        let mut preview = Preview::new(src.clone());
        let items = vec![Shape::Solid {
            rect: IRect::new(0, 0, 8, 8),
            color: Rgba::opaque(7, 7, 7),
        }];
        preview.render(&items, None);
        preview.reset();
        let back = preview.render(&[], None).clone();
        assert_eq!(back, src);
    }

    /// Moving drafts, removals, pushes, undos and an estimated text draft:
    /// every step must equal a from-scratch flatten (no ghost pixels from
    /// the dirty-rows fast path, no stale rows after undo).
    #[test]
    fn preview_dirty_draft_matches_full_flatten() {
        let src = checker(32, 32);
        let mut preview = Preview::new(src.clone());
        let items: Vec<Shape> = Vec::new();
        let ground = |items: &[Shape], draft: Option<&Shape>| -> Frame {
            let mut v = items.to_vec();
            if let Some(d) = draft {
                v.push(d.clone());
            }
            flatten(&src, &v)
        };

        // Growing pen draft (the mousemove hot path).
        let mut pts = vec![(2, 2)];
        for end in [(6, 6), (12, 4), (20, 18), (28, 28)] {
            pts.push(end);
            let draft = Shape::Pen {
                pts: pts.clone(),
                color: Rgba::opaque(255, 0, 0),
                width: 3,
            };
            assert_eq!(
                preview.render(&items, Some(&draft)).clone(),
                ground(&items, Some(&draft))
            );
        }
        // Draft removed: back to committed, no ghosts on the next stroke.
        assert_eq!(preview.render(&items, None).clone(), ground(&items, None));
        let draft2 = Shape::Line {
            a: (30, 2),
            b: (2, 30),
            color: Rgba::opaque(0, 255, 0),
            width: 4,
        };
        assert_eq!(
            preview.render(&items, Some(&draft2)).clone(),
            ground(&items, Some(&draft2))
        );
        assert_eq!(preview.render(&items, None).clone(), ground(&items, None));

        // Commit, undo, redraft: caches rebase without stale rows.
        let mut items = items;
        items.push(Shape::Rect {
            rect: IRect::new(4, 4, 8, 8),
            color: Rgba::opaque(0, 0, 255),
            width: 2,
            filled: true,
        });
        assert_eq!(preview.render(&items, None).clone(), ground(&items, None));
        items.pop();
        assert_eq!(preview.render(&items, None).clone(), ground(&items, None));
        let draft3 = Shape::Mosaic {
            rect: IRect::new(0, 0, 32, 32),
            block: 8,
        };
        assert_eq!(
            preview.render(&items, Some(&draft3)).clone(),
            ground(&items, Some(&draft3))
        );

        // Estimated text draft takes the full-refresh path; the stroke
        // after it must still be ghost-free.
        let text = Shape::Text {
            at: (2, 20),
            text: "Hi".to_string(),
            color: Rgba::opaque(255, 255, 255),
            size: 16,
        };
        assert_eq!(
            preview.render(&items, Some(&text)).clone(),
            ground(&items, Some(&text))
        );
        assert_eq!(preview.render(&items, None).clone(), ground(&items, None));
        let draft4 = Shape::Pen {
            pts: vec![(0, 31), (31, 0)],
            color: Rgba::opaque(255, 255, 0),
            width: 2,
        };
        assert_eq!(
            preview.render(&items, Some(&draft4)).clone(),
            ground(&items, Some(&draft4))
        );
    }

    #[test]
    fn copy_rows_copies_only_the_band() {
        let mut a = checker(10, 10);
        let b = a.clone();
        // Corrupt a band in `a`, restore it through copy_rows.
        for y in 3..6 {
            for x in 0..10 {
                let i = a.index(x, y).unwrap();
                a.px[i] = 1;
                a.px[i + 1] = 2;
                a.px[i + 2] = 3;
            }
        }
        super::copy_rows(&mut a, &b, &IRect::new(0, 3, 10, 3));
        assert_eq!(a, b);
        // Empty rect is a no-op (stays corrupted).
        for y in 7..9 {
            for x in 0..10 {
                let i = a.index(x, y).unwrap();
                a.px[i] = 9;
            }
        }
        let snapshot = a.clone();
        super::copy_rows(&mut a, &b, &IRect::EMPTY);
        assert_eq!(a, snapshot);
    }
}
