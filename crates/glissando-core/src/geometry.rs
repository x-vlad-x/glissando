//! Turns a [`Layer`] into concrete key rectangles and answers hit tests.
//!
//! Keys tile their row completely for hit testing (gaps belong to the nearest
//! key, the first/last key of a short row extends to the edge) while the
//! visual rectangle is inset by half the gap, mirroring how Gboard keeps
//! touch targets larger than the drawn keys.

use crate::layout::{KeyStyle, Layer};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    /// Squared distance from a point to the rectangle (0 inside).
    pub fn dist2(&self, px: f32, py: f32) -> f32 {
        let dx = (self.x - px).max(0.0).max(px - self.right());
        let dy = (self.y - py).max(0.0).max(py - self.bottom());
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryParams {
    /// Region the keys are laid out in (excludes the suggestion strip).
    pub area: Rect,
    pub row_height: f32,
    pub gap_x: f32,
    pub gap_y: f32,
    pub pad_x: f32,
    pub pad_top: f32,
    pub pad_bottom: f32,
}

impl GeometryParams {
    pub fn phone(width: f32, row_height: f32) -> GeometryParams {
        GeometryParams {
            area: Rect::new(0.0, 0.0, width, 0.0),
            row_height,
            gap_x: 6.0,
            gap_y: 9.0,
            pad_x: 3.0,
            pad_top: 4.0,
            pad_bottom: 6.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyGeom {
    pub row: usize,
    pub col: usize,
    pub hit: Rect,
    pub visual: Rect,
    pub style: KeyStyle,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerGeometry {
    pub keys: Vec<KeyGeom>,
    /// Width of one key unit in pixels.
    pub unit: f32,
    pub height: f32,
    pub row_bounds: Vec<(f32, f32)>,
}

impl LayerGeometry {
    pub fn compute(layer: &Layer, p: &GeometryParams) -> LayerGeometry {
        let units = layer.max_units().max(1.0);
        let inner_w = p.area.w - 2.0 * p.pad_x;
        let unit = inner_w / units;
        let mut keys = Vec::new();
        let mut row_bounds = Vec::new();
        let mut y = p.area.y + p.pad_top;
        let nrows = layer.rows.len();
        for (ri, row) in layer.rows.iter().enumerate() {
            let rh = p.row_height * row.height;
            // The hit area of the first/last row extends into the padding.
            let hit_top = if ri == 0 { p.area.y } else { y };
            let hit_bottom = if ri + 1 == nrows { y + rh + p.pad_bottom } else { y + rh };
            row_bounds.push((hit_top, hit_bottom));
            let row_w = row.units() * unit;
            let mut x = p.area.x + p.pad_x + (inner_w - row_w) / 2.0;
            let first_real = row.keys.iter().position(|k| k.style != KeyStyle::Invisible);
            let last_real = row.keys.iter().rposition(|k| k.style != KeyStyle::Invisible);
            for (ci, k) in row.keys.iter().enumerate() {
                let w = k.width * unit;
                if k.style != KeyStyle::Invisible {
                    let visual = Rect::new(x, y, w, rh).inset(p.gap_x / 2.0, p.gap_y / 2.0);
                    let mut hx0 = x;
                    let mut hx1 = x + w;
                    if Some(ci) == first_real {
                        hx0 = p.area.x;
                    }
                    if Some(ci) == last_real {
                        hx1 = p.area.right();
                    }
                    keys.push(KeyGeom {
                        row: ri,
                        col: ci,
                        hit: Rect::new(hx0, hit_top, hx1 - hx0, hit_bottom - hit_top),
                        visual,
                        style: k.style,
                    });
                }
                x += w;
            }
            y += rh;
        }
        LayerGeometry { keys, unit, height: y + p.pad_bottom - p.area.y, row_bounds }
    }

    /// Index into `keys` for the key under (x, y); points between keys or
    /// slightly outside resolve to the nearest key.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if let Some(i) = self.keys.iter().position(|k| k.hit.contains(x, y)) {
            return Some(i);
        }
        self.nearest(x, y)
    }

    pub fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        self.keys.iter().enumerate().min_by(|a, b| a.1.hit.dist2(x, y).total_cmp(&b.1.hit.dist2(x, y))).map(|(i, _)| i)
    }

    pub fn find(&self, row: usize, col: usize) -> Option<usize> {
        self.keys.iter().position(|k| k.row == row && k.col == col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{RowDef, parse_row};

    fn layer(rows: &[&str]) -> Layer {
        Layer { rows: rows.iter().map(|r| RowDef { keys: parse_row(r).unwrap(), height: 1.0 }).collect() }
    }

    #[test]
    fn tiles_and_hits() {
        let l = layer(&["q w e r t y u i o p", "{gap:0.5} a s d f g h j k l {gap:0.5}"]);
        let mut p = GeometryParams::phone(360.0, 50.0);
        p.area = Rect::new(0.0, 0.0, 360.0, 0.0);
        let g = LayerGeometry::compute(&l, &p);
        assert_eq!(g.keys.len(), 19);
        assert!((g.unit - 35.4).abs() < 0.01);
        // Left edge of the indented row resolves to 'a'.
        let a = g.hit(1.0, 4.0 + 50.0 + 25.0).unwrap();
        assert_eq!((g.keys[a].row, g.keys[a].col), (1, 1));
        // Gap between visual keys still hits a key.
        let q = &g.keys[0];
        let between = q.visual.right() + 1.0;
        let hit = g.hit(between, q.visual.center().1).unwrap();
        assert!(hit == 0 || hit == 1);
        // Above the keyboard resolves to the top row.
        assert_eq!(g.keys[g.hit(100.0, -5.0).unwrap()].row, 0);
        assert!((g.height - (4.0 + 100.0 + 6.0)).abs() < 0.01);
    }
}
