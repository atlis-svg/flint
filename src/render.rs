//! On-screen drawing of document shapes through egui's painter.

use crate::model::*;
use egui::epaint::{Mesh, PathShape, PathStroke, TextShape};
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Stroke, Vec2};

#[derive(Clone, Copy)]
pub struct View {
    pub origin: Pos2, // screen position of stage (0,0)
    pub zoom: f32,
}

impl View {
    pub fn to_screen(&self, p: P) -> Pos2 {
        Pos2::new(self.origin.x + p[0] * self.zoom, self.origin.y + p[1] * self.zoom)
    }
    pub fn to_stage(&self, p: Pos2) -> P {
        [(p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom]
    }
}

pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

pub fn rgba(c: Color32) -> Rgba {
    c.to_srgba_unmultiplied()
}

fn mul_alpha(c: Rgba, a: f32) -> Color32 {
    let mut c = c;
    c[3] = (c[3] as f32 * a) as u8;
    c32(c)
}

pub struct Style {
    pub alpha: f32,
    pub outline: Option<Color32>,
}

pub fn draw_shape(p: &Painter, v: &View, s: &Shape, st: &Style) {
    if s.is_container() {
        for leaf in s.flatten() {
            draw_shape(p, v, &leaf, st);
        }
        return;
    }
    if let Geom::Text { text, size } = &s.geom {
        if let Some(col) = st.outline {
            let (pts, _) = s.world_outline();
            let sp: Vec<Pos2> = pts.iter().map(|q| v.to_screen(*q)).collect();
            p.add(PathShape::closed_line(sp, Stroke::new(1.0, col)));
            return;
        }
        let Some(fill) = s.fill else { return };
        let px = size * s.xf.sy.abs() * v.zoom;
        if px < 1.0 {
            return;
        }
        let galley = p.layout_no_wrap(text.clone(), FontId::new(px, FontFamily::Proportional), mul_alpha(fill, st.alpha));
        let pos = v.to_screen([s.xf.x, s.xf.y]);
        p.add(TextShape::new(pos, galley, mul_alpha(fill, st.alpha)).with_angle(s.xf.rot));
        return;
    }
    let (pts, closed) = s.world_outline();
    if pts.is_empty() {
        return;
    }
    let sp: Vec<Pos2> = pts.iter().map(|q| v.to_screen(*q)).collect();
    if let Some(col) = st.outline {
        if closed {
            p.add(PathShape::closed_line(sp, Stroke::new(1.0, col)));
        } else {
            p.add(PathShape::line(sp, Stroke::new(1.0, col)));
        }
        return;
    }
    if closed && sp.len() >= 3 {
        if let Some(fill) = s.fill {
            fill_polygon(p, &sp, mul_alpha(fill, st.alpha));
        }
    }
    if let Some(sc) = s.stroke {
        let w = (s.stroke_w * s.xf.avg_scale() * v.zoom).max(0.5);
        let col = mul_alpha(sc, st.alpha);
        stroke_polyline(p, &sp, closed, w, col);
    }
}

/// Stroke with round joins/caps (egui strokes are miter-free feathered lines; add dots at the joints).
pub fn stroke_polyline(p: &Painter, sp: &[Pos2], closed: bool, w: f32, col: Color32) {
    if sp.len() == 1 {
        p.circle_filled(sp[0], w * 0.5, col);
        return;
    }
    let shape = if closed {
        PathShape { points: sp.to_vec(), closed: true, fill: Color32::TRANSPARENT, stroke: PathStroke::new(w, col) }
    } else {
        PathShape::line(sp.to_vec(), PathStroke::new(w, col))
    };
    p.add(shape);
    if w > 3.0 && col.a() == 255 {
        if !closed {
            p.circle_filled(sp[0], w * 0.5, col);
            p.circle_filled(sp[sp.len() - 1], w * 0.5, col);
        }
        if sp.len() < 400 {
            for q in &sp[1..sp.len() - 1] {
                p.circle_filled(*q, w * 0.5, col);
            }
        }
    }
}

pub fn fill_polygon(p: &Painter, sp: &[Pos2], col: Color32) {
    let tris = triangulate(sp);
    let mut mesh = Mesh::default();
    for q in sp {
        mesh.colored_vertex(*q, col);
    }
    for t in tris {
        mesh.add_triangle(t[0], t[1], t[2]);
    }
    p.add(mesh);
}

/// Ear-clipping triangulation of a simple polygon; falls back to a fan if it gets stuck.
pub fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    if n < 3 {
        return out;
    }
    let area: f32 = (0..n).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        a.x * b.y - b.x * a.y
    }).sum();
    let mut idx: Vec<usize> = (0..n).collect();
    if area < 0.0 {
        idx.reverse();
    }
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            let cr = cross(a, b, c);
            if cr <= 1e-7 {
                if cr.abs() <= 1e-7 {
                    // degenerate vertex: drop it
                    idx.remove(i);
                    clipped = true;
                    break;
                }
                continue;
            }
            let contains = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = pts[j];
                cross(a, b, q) > 0.0 && cross(b, c, q) > 0.0 && cross(c, a, q) > 0.0
            });
            if contains {
                continue;
            }
            out.push([ia as u32, ib as u32, ic as u32]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // self-intersecting: fan the rest
            for i in 1..idx.len() - 1 {
                out.push([idx[0] as u32, idx[i] as u32, idx[i + 1] as u32]);
            }
            return out;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    }
    out
}

pub fn dashed_rect(p: &Painter, r: egui::Rect, col: Color32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, col), 4.0, 3.0));
}

pub fn v2(p: P) -> Vec2 {
    Vec2::new(p[0], p[1])
}
