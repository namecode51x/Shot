//! Annotation model and undo/redo history.
//!
//! Pure data: no OS calls, no rasterization. Coordinates are in **image
//! pixels** of the cropped selection. Rendering happens in [`crate::flatten`],
//! always in list order, so a redaction placed after a shape also destroys
//! that shape's pixels.

use crate::image::{IRect, Rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Line,
    Arrow,
    Rect,
    Box,
    Text,
    Highlight,
    Mosaic,
    Solid,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Pen {
        pts: Vec<(i32, i32)>,
        color: Rgba,
        width: u32,
    },
    Line {
        a: (i32, i32),
        b: (i32, i32),
        color: Rgba,
        width: u32,
    },
    Arrow {
        a: (i32, i32),
        b: (i32, i32),
        color: Rgba,
        width: u32,
    },
    Rect {
        rect: IRect,
        color: Rgba,
        width: u32,
        filled: bool,
    },
    Text {
        at: (i32, i32),
        text: String,
        color: Rgba,
        size: u32,
    },
    Highlight {
        a: (i32, i32),
        b: (i32, i32),
        color: Rgba,
        width: u32,
    },
    Mosaic {
        rect: IRect,
        block: u32,
    },
    Solid {
        rect: IRect,
        color: Rgba,
    },
}

impl Shape {
    /// Shift every coordinate by `(dx, dy)`. Used when the editor crop
    /// window moves: annotations stay glued to the same photo pixels.
    pub fn translate(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        let shift = |(x, y): (i32, i32)| (x + dx, y + dy);
        match self {
            Shape::Pen { pts, .. } => {
                for p in pts.iter_mut() {
                    *p = shift(*p);
                }
            }
            Shape::Line { a, b, .. }
            | Shape::Arrow { a, b, .. }
            | Shape::Highlight { a, b, .. } => {
                *a = shift(*a);
                *b = shift(*b);
            }
            Shape::Rect { rect, .. }
            | Shape::Mosaic { rect, .. }
            | Shape::Solid { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Shape::Text { at, .. } => {
                *at = shift(*at);
            }
        }
    }
    /// Bounding box in image space (used for partial invalidation).
    pub fn bounds(&self) -> IRect {
        match self {
            Shape::Pen { pts, width, .. } => {
                let mut r = IRect::EMPTY;
                for &(x, y) in pts {
                    let p = IRect::from_points(x, y, x, y).expand(*width as i32 + 1);
                    r = if r.is_empty() {
                        p
                    } else {
                        IRect::new(
                            r.x.min(p.x),
                            r.y.min(p.y),
                            r.right().max(p.right()) - r.x.min(p.x),
                            r.bottom().max(p.bottom()) - r.y.min(p.y),
                        )
                    };
                }
                r
            }
            Shape::Line { a, b, width, .. }
            | Shape::Arrow { a, b, width, .. }
            | Shape::Highlight { a, b, width, .. } => {
                IRect::from_points(a.0, a.1, b.0, b.1).expand(*width as i32 + 4)
            }
            Shape::Rect { rect, width, .. } => rect.expand(*width as i32 + 1),
            Shape::Text { at, size, text, .. } => {
                // Approximation: text may extend right/down from the anchor.
                let est_w = (text.chars().count() as i32) * (*size as i32) / 2 + 4;
                IRect::new(at.0 - 2, at.1 - *size as i32, est_w + 4, *size as i32 * 2)
            }
            Shape::Mosaic { rect, .. } | Shape::Solid { rect, .. } => *rect,
        }
    }
}

pub const MAX_HISTORY: usize = 64;

/// Ordered annotation list with bounded undo/redo.
///
/// Every mutation appends/removes **at the tail**, which is the invariant
/// [`crate::flatten::Preview`] relies on for incremental re-rendering.
#[derive(Debug, Clone)]
pub struct History {
    items: Vec<Shape>,
    undo_stack: Vec<Vec<Shape>>,
    redo_stack: Vec<Vec<Shape>>,
}

impl Default for History {
    fn default() -> Self {
        History {
            items: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[Shape] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// True when the document differs from the untouched capture.
    pub fn is_dirty(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn push(&mut self, shape: Shape) {
        self.snapshot_for_push();
        self.items.push(shape);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        match self.undo_stack.pop() {
            Some(state) => {
                self.redo_stack.push(std::mem::take(&mut self.items));
                self.items = state;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo_stack.pop() {
            Some(state) => {
                self.undo_stack.push(std::mem::take(&mut self.items));
                self.items = state;
                true
            }
            None => false,
        }
    }

    /// Shift committed items and every undo/redo snapshot by `(dx, dy)`.
    /// Keeps undo/redo coherent across an editor crop-window move (every
    /// snapshot describes the same photo pixels, just rebased).
    pub fn translate_all(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        for s in self.items.iter_mut() {
            s.translate(dx, dy);
        }
        for snap in self
            .undo_stack
            .iter_mut()
            .chain(self.redo_stack.iter_mut())
        {
            for s in snap.iter_mut() {
                s.translate(dx, dy);
            }
        }
    }

    fn snapshot_for_push(&mut self) {
        self.undo_stack.push(self.items.clone());
        if self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: i32) -> Shape {
        Shape::Line {
            a: (x, 0),
            b: (x, 10),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        }
    }

    #[test]
    fn push_undo_redo_keeps_order() {
        let mut h = History::new();
        assert!(!h.is_dirty());
        h.push(line(1));
        h.push(line(2));
        h.push(line(3));
        assert_eq!(h.len(), 3);
        assert!(h.undo());
        assert_eq!(h.len(), 2);
        assert!(h.undo());
        assert_eq!(h.len(), 1);
        assert!(h.redo());
        assert_eq!(h.len(), 2);
        assert_eq!(h.items()[1], line(2));
        assert!(h.redo());
        assert_eq!(h.len(), 3);
        assert!(!h.can_redo());
        assert!(h.undo());
    }

    #[test]
    fn push_after_undo_clears_redo() {
        let mut h = History::new();
        h.push(line(1));
        h.push(line(2));
        assert!(h.undo());
        assert!(h.can_redo());
        h.push(line(9));
        assert!(!h.can_redo());
        assert_eq!(h.len(), 2);
        assert_eq!(h.items()[1], line(9));
    }

    #[test]
    fn undo_on_empty_is_false() {
        let mut h = History::new();
        assert!(!h.undo());
        assert!(!h.redo());
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new();
        for i in 0..(MAX_HISTORY + 20) as i32 {
            h.push(line(i));
        }
        let mut undos = 0;
        while h.undo() {
            undos += 1;
        }
        assert_eq!(undos, MAX_HISTORY);
    }

    #[test]
    fn shape_bounds_cover_geometry() {
        let s = Shape::Rect {
            rect: IRect::new(5, 6, 10, 4),
            color: Rgba::opaque(1, 2, 3),
            width: 3,
            filled: false,
        };
        let b = s.bounds();
        assert!(b.contains(5, 6) && b.contains(14, 9));
        let p = Shape::Pen {
            pts: vec![(0, 0), (20, 5)],
            color: Rgba::opaque(0, 0, 0),
            width: 2,
        };
        let b = p.bounds();
        assert!(b.x <= 0 && b.right() >= 20);
    }

    #[test]
    fn translate_shifts_every_coordinate_kind() {
        let red = Rgba::opaque(255, 0, 0);
        let mut s = Shape::Line {
            a: (10, 20),
            b: (30, 40),
            color: red,
            width: 2,
        };
        s.translate(5, -7);
        assert_eq!(
            s,
            Shape::Line {
                a: (15, 13),
                b: (35, 33),
                color: red,
                width: 2
            }
        );
        let mut s = Shape::Pen {
            pts: vec![(0, 0), (4, 6)],
            color: red,
            width: 3,
        };
        s.translate(-2, 2);
        assert_eq!(
            s,
            Shape::Pen {
                pts: vec![(-2, 2), (2, 8)],
                color: red,
                width: 3
            }
        );
        let mut s = Shape::Rect {
            rect: IRect::new(10, 20, 30, 40),
            color: red,
            width: 2,
            filled: false,
        };
        s.translate(-3, 9);
        assert_eq!(
            s,
            Shape::Rect {
                rect: IRect::new(7, 29, 30, 40),
                color: red,
                width: 2,
                filled: false
            }
        );
        let mut s = Shape::Text {
            at: (1, 1),
            text: "hi".to_string(),
            color: red,
            size: 16,
        };
        s.translate(2, 3);
        assert_eq!(
            s,
            Shape::Text {
                at: (3, 4),
                text: "hi".to_string(),
                color: red,
                size: 16
            }
        );
        // Zero shift is a no-op (also for empty pen paths).
        let mut s = Shape::Mosaic {
            rect: IRect::new(0, 0, 8, 8),
            block: 4,
        };
        let before = s.clone();
        s.translate(0, 0);
        assert_eq!(s, before);
    }

    #[test]
    fn translate_all_rebases_items_and_snapshots() {
        let l1 = Shape::Line {
            a: (0, 0),
            b: (10, 10),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        };
        let l2 = Shape::Rect {
            rect: IRect::new(20, 20, 10, 10),
            color: Rgba::opaque(0, 255, 0),
            width: 3,
            filled: true,
        };
        let mut h = History::new();
        h.push(l1.clone());
        h.push(l2.clone());
        h.translate_all(100, 50);
        let t1 = Shape::Line {
            a: (100, 50),
            b: (110, 60),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        };
        let t2 = Shape::Rect {
            rect: IRect::new(120, 70, 10, 10),
            color: Rgba::opaque(0, 255, 0),
            width: 3,
            filled: true,
        };
        assert_eq!(h.items(), &[t1.clone(), t2.clone()]);
        // Undo snapshots were rebased too: undo restores the shifted past,
        // redo returns to the shifted present.
        assert!(h.undo());
        assert_eq!(h.items(), &[t1.clone()]);
        assert!(h.redo());
        assert_eq!(h.items(), &[t1, t2]);
    }
}
