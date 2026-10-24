//! Paint-bucket region fill: rasterize the lines around a click, flood-fill the enclosed area,
//! and trace it back into a polygon (inner shapes become holes, joined to the outline by a slit).

use crate::export;
use crate::model::*;
use std::collections::{HashMap, VecDeque};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Pixmap, Stroke, Transform};

/// Most pixels the fill mask may use; larger areas are rasterized more coarsely.
const MAX_PIXELS: f32 = 4_000_000.0;

#[derive(Debug, PartialEq)]
pub enum FillError {
    /// The click is on a line or filled shape, not in an empty area.
    OnShape,
    /// The area leaks out to open space (the lines around it don't close it off).
    NotEnclosed,
}

/// Finds the empty area around `click` bounded by `barriers` (world-space leaf shapes).
/// `px_per_unit` is the mask resolution (the editor zoom, so one pixel ≈ one screen pixel);
/// `gap_px` closes gaps of up to about that many pixels between lines.
/// Returns a closed polygon in stage coordinates.
pub fn region_fill(barriers: &[Shape], click: P, px_per_unit: f32, gap_px: f32) -> Result<Vec<P>, FillError> {
    let Some(b) = barriers.iter().map(|s| s.bounds()).reduce(union_bounds) else { return Err(FillError::NotEnclosed) };
    if click[0] <= b[0] || click[1] <= b[1] || click[0] >= b[2] || click[1] >= b[3] {
        return Err(FillError::NotEnclosed);
    }
    let margin = 4.0 + gap_px;
    let (bw, bh) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
    let mut s = px_per_unit.max(0.05);
    let area = (bw * s + 2.0 * margin) * (bh * s + 2.0 * margin);
    if area > MAX_PIXELS {
        s *= (MAX_PIXELS / area).sqrt();
    }
    let w = (bw * s + 2.0 * margin).ceil() as u32;
    let h = (bh * s + 2.0 * margin).ceil() as u32;
    let to_px = |p: P| [(p[0] - b[0]) * s + margin, (p[1] - b[1]) * s + margin];
    let to_stage = |q: [f32; 2]| [(q[0] - margin) / s + b[0], (q[1] - margin) / s + b[1]];
    let tf = Transform::from_row(s, 0.0, 0.0, s, margin - b[0] * s, margin - b[1] * s);

    // 1. barrier mask: lines (thickened to close gaps), filled shapes and text
    let mut pm = Pixmap::new(w, h).ok_or(FillError::NotEnclosed)?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    paint.anti_alias = false;
    for sh in barriers {
        if sh.is_text() {
            if let Some(path) = export::text_path(sh) {
                pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
            }
            continue;
        }
        let (pts, closed) = sh.world_outline();
        let Some(path) = export::poly_path(&pts, closed) else { continue };
        if closed && sh.fill.is_some_and(|c| c[3] > 0) {
            pm.fill_path(&path, &paint, FillRule::EvenOdd, tf, None);
        }
        if sh.stroke.is_some_and(|c| c[3] > 0) || !closed {
            let width = (sh.stroke_w * sh.xf.avg_scale() * s).max(1.0) + 2.0 * gap_px;
            let st = Stroke { width: width / s, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
            pm.stroke_path(&path, &paint, &st, tf, None);
        }
    }
    let (wu, hu) = (w as usize, h as usize);
    let wall: Vec<bool> = pm.pixels().iter().map(|p| p.alpha() > 0).collect();

    // 2. flood fill from the click
    let c = to_px(click);
    let (cx, cy) = (c[0] as usize, c[1] as usize);
    if cx >= wu || cy >= hu || wall[cy * wu + cx] {
        return Err(FillError::OnShape);
    }
    let mut inside = vec![false; wu * hu];
    let mut queue = VecDeque::from([(cx, cy)]);
    inside[cy * wu + cx] = true;
    while let Some((x, y)) = queue.pop_front() {
        if x == 0 || y == 0 || x + 1 == wu || y + 1 == hu {
            return Err(FillError::NotEnclosed);
        }
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            let i = ny * wu + nx;
            if !inside[i] && !wall[i] {
                inside[i] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    // 3. grow the region under the surrounding lines so no sliver shows between fill and stroke
    let grow = (gap_px + 1.0).round() as usize;
    for _ in 0..grow {
        let prev = inside.clone();
        for y in 1..hu - 1 {
            for x in 1..wu - 1 {
                let i = y * wu + x;
                if !prev[i] && (prev[i - 1] || prev[i + 1] || prev[i - wu] || prev[i + wu]) {
                    inside[i] = true;
                }
            }
        }
    }

    // 4. trace boundary loops along pixel edges, simplify, and stitch holes into the outline
    let mut loops = trace_loops(&inside, wu, hu);
    if loops.is_empty() {
        return Err(FillError::NotEnclosed);
    }
    loops.sort_by(|a, b| signed_area(b).abs().total_cmp(&signed_area(a).abs()));
    let eps = 0.9;
    let outer = simplify_loop(&loops[0], eps);
    if outer.len() < 3 {
        return Err(FillError::NotEnclosed);
    }
    let holes: Vec<Vec<[f32; 2]>> = loops[1..].iter().map(|l| simplify_loop(l, eps)).filter(|l| l.len() >= 3 && signed_area(l).abs() > 4.0).collect();
    let ring = bridge_holes(outer, holes);
    Ok(ring.into_iter().map(to_stage).collect())
}

/// Directed edges around every inside pixel (inside on the right in y-down coords), linked into loops.
fn trace_loops(inside: &[bool], w: usize, h: usize) -> Vec<Vec<[f32; 2]>> {
    let at = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && inside[y as usize * w + x as usize];
    let mut next: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for y in 0..h as isize {
        for x in 0..w as isize {
            if !at(x, y) {
                continue;
            }
            let (x0, y0, x1, y1) = (x as i32, y as i32, x as i32 + 1, y as i32 + 1);
            if !at(x, y - 1) {
                next.entry((x0, y0)).or_default().push((x1, y0));
            }
            if !at(x + 1, y) {
                next.entry((x1, y0)).or_default().push((x1, y1));
            }
            if !at(x, y + 1) {
                next.entry((x1, y1)).or_default().push((x0, y1));
            }
            if !at(x - 1, y) {
                next.entry((x0, y1)).or_default().push((x0, y0));
            }
        }
    }
    let mut loops = Vec::new();
    let starts: Vec<(i32, i32)> = next.keys().copied().collect();
    for start in starts {
        while let Some(first) = next.get_mut(&start).and_then(|v| v.pop()) {
            let mut pts = vec![[start.0 as f32, start.1 as f32]];
            let mut cur = first;
            let mut guard = 0;
            while cur != start && guard < 50_000_000 {
                guard += 1;
                pts.push([cur.0 as f32, cur.1 as f32]);
                match next.get_mut(&cur).and_then(|v| v.pop()) {
                    Some(n) => cur = n,
                    None => break,
                }
            }
            if pts.len() >= 4 {
                loops.push(pts);
            }
        }
    }
    loops
}

fn signed_area(l: &[[f32; 2]]) -> f32 {
    let n = l.len();
    (0..n).map(|i| l[i][0] * l[(i + 1) % n][1] - l[(i + 1) % n][0] * l[i][1]).sum::<f32>() * 0.5
}

fn simplify_loop(l: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let mut open = l.to_vec();
    open.push(l[0]);
    let mut s = simplify(&open, eps);
    s.pop();
    s
}

/// Joins each hole to the outline with a zero-width slit so the region is one polygon
/// (filled correctly by even-odd rasterizers and the ear-clipping triangulator).
fn bridge_holes(mut outer: Vec<[f32; 2]>, mut holes: Vec<Vec<[f32; 2]>>) -> Vec<[f32; 2]> {
    // rightmost holes first, like earcut, so bridges rarely cross other holes
    holes.sort_by(|a, b| {
        let mx = |h: &Vec<[f32; 2]>| h.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        mx(b).total_cmp(&mx(a))
    });
    for hole in holes {
        let hi = (0..hole.len()).max_by(|&a, &b| hole[a][0].total_cmp(&hole[b][0])).unwrap();
        let hp = hole[hi];
        let oi = (0..outer.len())
            .min_by(|&a, &b| {
                let d = |p: [f32; 2]| (p[0] - hp[0]).powi(2) + (p[1] - hp[1]).powi(2);
                d(outer[a]).total_cmp(&d(outer[b]))
            })
            .unwrap();
        // The way back runs a hair beside the way in, so the polygon stays simple (no repeated
        // points), which ear clipping needs: each duplicate slides a little along its next edge.
        let nudge = |p: [f32; 2], toward: [f32; 2]| {
            let (dx, dy) = (toward[0] - p[0], toward[1] - p[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let t = (0.05 / len).min(0.25);
            [p[0] + dx * t, p[1] + dy * t]
        };
        let hole_back = nudge(hole[hi], hole[(hi + hole.len() - 1) % hole.len()]);
        let outer_back = nudge(outer[oi], outer[(oi + 1) % outer.len()]);
        let mut ring = Vec::with_capacity(outer.len() + hole.len() + 2);
        ring.extend_from_slice(&outer[..=oi]);
        for k in 0..hole.len() {
            ring.push(hole[(hi + k) % hole.len()]);
        }
        ring.push(hole_back);
        ring.push(outer_back);
        ring.extend_from_slice(&outer[oi + 1..]);
        outer = ring;
    }
    outer
}
