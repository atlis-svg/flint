//! Document model: layers, keyframes, shapes, tweens and frame evaluation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f32::consts::TAU;
use std::sync::OnceLock;

pub type Rgba = [u8; 4];
pub type P = [f32; 2];

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub width: f32,
    pub height: f32,
    pub fps: f32,
    pub background: Rgba,
    pub layers: Vec<Layer>, // index 0 is the top layer (drawn last)
    pub next_id: u64,
    /// Runs once when the movie starts.
    #[serde(default)]
    pub script: String,
    /// Runs every tick at the frame rate, even while stopped (like enterFrame).
    #[serde(default)]
    pub on_frame: String,
    /// Runs on every key press; the pressed key's name is in `key`.
    #[serde(default)]
    pub on_key: String,
    /// Flint Hub movie this document was published as (publishing again updates it).
    #[serde(default)]
    pub hub_id: Option<i64>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Functions available to every script in the movie.
    #[serde(default)]
    pub functions: String,
    #[serde(skip)]
    pub edit_stack: Vec<EditCtx>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub outline: bool,
    pub color: Rgba,
    pub len: usize,
    pub keys: BTreeMap<usize, Keyframe>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Keyframe {
    pub shapes: Vec<Shape>,
    pub tween: Tween,
    #[serde(default)]
    pub label: String,
    /// Frame actions, run when the playhead enters this keyframe.
    #[serde(default)]
    pub script: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Debug)]
pub enum TweenKind {
    #[default]
    None,
    Classic,
    Shape,
}

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Tween {
    pub kind: TweenKind,
    /// -100 (ease in) .. 100 (ease out)
    pub ease: f32,
    /// Extra full rotations over the tween, signed (positive = clockwise).
    pub spins: i32,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xform {
    pub x: f32,
    pub y: f32,
    pub sx: f32,
    pub sy: f32,
    pub rot: f32,
}

impl Default for Xform {
    fn default() -> Self {
        Xform { x: 0.0, y: 0.0, sx: 1.0, sy: 1.0, rot: 0.0 }
    }
}

impl Xform {
    pub fn at(x: f32, y: f32) -> Self {
        Xform { x, y, ..Default::default() }
    }
    pub fn apply(&self, p: P) -> P {
        let (s, c) = self.rot.sin_cos();
        let (lx, ly) = (p[0] * self.sx, p[1] * self.sy);
        [self.x + lx * c - ly * s, self.y + lx * s + ly * c]
    }
    pub fn inverse(&self, p: P) -> P {
        let (s, c) = self.rot.sin_cos();
        let (dx, dy) = (p[0] - self.x, p[1] - self.y);
        let lx = dx * c + dy * s;
        let ly = -dx * s + dy * c;
        [lx / nz(self.sx), ly / nz(self.sy)]
    }
    pub fn avg_scale(&self) -> f32 {
        ((self.sx.abs() + self.sy.abs()) * 0.5).max(0.001)
    }
}

fn nz(v: f32) -> f32 {
    if v.abs() < 1e-4 { 1e-4f32.copysign(v) } else { v }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Geom {
    Path { pts: Vec<P>, closed: bool },
    Rect { w: f32, h: f32, radius: f32 },
    Ellipse { rx: f32, ry: f32 },
    Text { text: String, size: f32 },
    /// Several objects merged into one; children are in the group's local space.
    Group { children: Vec<Shape> },
    /// An instance of a library symbol. `children` is filled in when a frame is evaluated.
    Symbol {
        symbol: u64,
        #[serde(default)]
        first: usize,
        #[serde(default)]
        mode: SymMode,
        #[serde(skip)]
        children: Vec<Shape>,
    },
}

/// How a symbol instance's own timeline plays.
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Debug)]
pub enum SymMode {
    #[default]
    Loop,
    Once,
    Single,
}

/// A reusable object with its own timeline (like an Animate movie clip).
#[derive(Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: u64,
    pub name: String,
    pub layers: Vec<Layer>,
}

impl Symbol {
    pub fn frame_count(&self) -> usize {
        self.layers.iter().map(|l| l.len).max().unwrap_or(1).max(1)
    }

    pub fn label_frame(&self, label: &str) -> Option<usize> {
        self.layers.iter().find_map(|l| l.keys.iter().find(|(_, k)| k.label == label).map(|(f, _)| *f))
    }
}

/// Where the editor is while editing inside a symbol (the symbol's layers are swapped into the document).
#[derive(Clone)]
pub struct EditCtx {
    pub symbol: u64,
    pub frame: usize,
    pub layer: usize,
    pub pan: [f32; 2],
    pub zoom: f32,
}

/// Frame of an instance's timeline, `local` frames after its keyframe started.
pub fn instance_frame(mode: SymMode, first: usize, local: usize, len: usize) -> usize {
    let len = len.max(1);
    match mode {
        SymMode::Loop => (first + local) % len,
        SymMode::Once => (first + local).min(len - 1),
        SymMode::Single => first.min(len - 1),
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Shape {
    pub id: u64,
    pub geom: Geom,
    pub xf: Xform,
    pub stroke: Option<Rgba>,
    pub stroke_w: f32,
    pub fill: Option<Rgba>,
    /// Instance name, used by scripts.
    #[serde(default)]
    pub name: String,
    /// Script run when this object is clicked in the player.
    #[serde(default)]
    pub on_click: String,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            id: 0,
            geom: Geom::Path { pts: Vec::new(), closed: false },
            xf: Xform::default(),
            stroke: None,
            stroke_w: 1.0,
            fill: None,
            name: String::new(),
            on_click: String::new(),
        }
    }
}

impl Shape {
    /// Outline in local coordinates; (points, closed).
    pub fn local_outline(&self) -> (Vec<P>, bool) {
        match &self.geom {
            Geom::Path { pts, closed } => (pts.clone(), *closed),
            Geom::Rect { w, h, radius } => (rect_outline(*w, *h, *radius), true),
            Geom::Ellipse { rx, ry } => {
                let n = 72;
                let pts = (0..n)
                    .map(|i| {
                        let a = i as f32 / n as f32 * TAU;
                        [a.cos() * rx, a.sin() * ry]
                    })
                    .collect();
                (pts, true)
            }
            Geom::Text { text, size } => {
                let (w, h) = text_extent(text, *size);
                (vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], true)
            }
            Geom::Group { children } | Geom::Symbol { children, .. } => {
                let b = children.iter().map(|c| c.bounds()).reduce(union_bounds).unwrap_or([0.0; 4]);
                (vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]], true)
            }
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.geom, Geom::Group { .. })
    }

    pub fn children(&self) -> Option<&Vec<Shape>> {
        match &self.geom {
            Geom::Group { children } | Geom::Symbol { children, .. } => Some(children),
            _ => None,
        }
    }

    /// Groups and symbol instances.
    pub fn is_container(&self) -> bool {
        self.children().is_some()
    }

    pub fn symbol_id(&self) -> Option<u64> {
        match self.geom {
            Geom::Symbol { symbol, .. } => Some(symbol),
            _ => None,
        }
    }

    /// Leaf shapes in world space (groups expanded, their transform baked in).
    pub fn flatten(&self) -> Vec<Shape> {
        let Some(children) = self.children() else { return vec![self.clone()] };
        let g = self.xf;
        let mut out = Vec::new();
        for c in children {
            for leaf in c.flatten() {
                out.push(match &leaf.geom {
                    Geom::Text { .. } => Shape { xf: compose(&g, &leaf.xf), ..leaf },
                    _ => {
                        let (pts, closed) = leaf.world_outline();
                        Shape {
                            geom: Geom::Path { pts: pts.into_iter().map(|p| g.apply(p)).collect(), closed },
                            xf: Xform::default(),
                            stroke_w: leaf.stroke_w * leaf.xf.avg_scale() * g.avg_scale(),
                            ..leaf
                        }
                    }
                });
            }
        }
        out
    }

    /// Applies `f` to every leaf (recursing into groups).
    pub fn for_leaves_mut(&mut self, f: &mut impl FnMut(&mut Shape)) {
        match &mut self.geom {
            Geom::Group { children } | Geom::Symbol { children, .. } => children.iter_mut().for_each(|c| c.for_leaves_mut(f)),
            _ => f(self),
        }
    }

    pub fn world_outline(&self) -> (Vec<P>, bool) {
        let (pts, closed) = self.local_outline();
        (pts.into_iter().map(|p| self.xf.apply(p)).collect(), closed)
    }

    pub fn bounds(&self) -> [f32; 4] {
        if self.is_container() {
            return self.flatten().iter().map(|c| c.bounds()).reduce(union_bounds).unwrap_or([self.xf.x, self.xf.y, self.xf.x, self.xf.y]);
        }
        let (pts, _) = self.world_outline();
        let mut b = bounds_of(&pts);
        let pad = if self.stroke.is_some() { self.stroke_w * self.xf.avg_scale() * 0.5 } else { 0.0 };
        b[0] -= pad;
        b[1] -= pad;
        b[2] += pad;
        b[3] += pad;
        b
    }

    pub fn is_text(&self) -> bool {
        matches!(self.geom, Geom::Text { .. })
    }

    /// Hit test in world coordinates. `tol` is extra slop in world units.
    pub fn hit(&self, p: P, tol: f32) -> bool {
        if self.is_container() {
            return self.flatten().iter().any(|c| c.hit(p, tol));
        }
        let (pts, closed) = self.world_outline();
        if pts.is_empty() {
            return false;
        }
        if (closed && (self.fill.is_some() || self.is_text())) && point_in_poly(p, &pts) {
            return true;
        }
        let half = self.stroke_w * self.xf.avg_scale() * 0.5 + tol;
        polyline_dist(p, &pts, closed) <= half
    }

    /// Inside test regardless of fill (used by the paint bucket).
    pub fn encloses(&self, p: P) -> bool {
        if self.is_container() {
            return self.flatten().iter().any(|c| c.encloses(p));
        }
        let (pts, closed) = self.world_outline();
        closed && point_in_poly(p, &pts)
    }
}

/// Transform of a child placed inside a parent transform (exact for uniform parent scale).
pub fn compose(g: &Xform, c: &Xform) -> Xform {
    let p = g.apply([c.x, c.y]);
    let flip = g.sx.signum() * g.sy.signum();
    Xform { x: p[0], y: p[1], sx: g.sx * c.sx, sy: g.sy * c.sy, rot: g.rot + c.rot * flip }
}

pub fn rect_outline(w: f32, h: f32, radius: f32) -> Vec<P> {
    let (hw, hh) = (w * 0.5, h * 0.5);
    let r = radius.clamp(0.0, hw.abs().min(hh.abs()));
    if r <= 0.01 {
        return vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]];
    }
    let mut out = Vec::new();
    let corners = [(hw - r, -hh + r, -0.25), (hw - r, hh - r, 0.0), (-hw + r, hh - r, 0.25), (-hw + r, -hh + r, 0.5)];
    for (cx, cy, start) in corners {
        for i in 0..=8 {
            let a = (start + i as f32 / 8.0 * 0.25) * TAU;
            out.push([cx + a.cos() * r, cy + a.sin() * r]);
        }
    }
    out
}

pub fn bounds_of(pts: &[P]) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in pts {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    if pts.is_empty() { [0.0; 4] } else { b }
}

pub fn union_bounds(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

pub fn point_in_poly(p: P, pts: &[P]) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn seg_dist(p: P, a: P, b: P) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let len2 = abx * abx + aby * aby;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a[0] + abx * t - p[0], a[1] + aby * t - p[1]);
    (cx * cx + cy * cy).sqrt()
}

pub fn polyline_dist(p: P, pts: &[P], closed: bool) -> f32 {
    if pts.len() == 1 {
        return seg_dist(p, pts[0], pts[0]);
    }
    let mut d = f32::MAX;
    for w in pts.windows(2) {
        d = d.min(seg_dist(p, w[0], w[1]));
    }
    if closed && pts.len() > 2 {
        d = d.min(seg_dist(p, pts[pts.len() - 1], pts[0]));
    }
    d
}

/// Ramer–Douglas–Peucker simplification.
pub fn simplify(pts: &[P], eps: f32) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let mut best = (0.0, 0);
        for i in a + 1..b {
            let d = seg_dist(pts[i], pts[a], pts[b]);
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Chaikin smoothing, keeps endpoints for open paths.
pub fn chaikin(pts: &[P], closed: bool, iters: usize) -> Vec<P> {
    let mut cur = pts.to_vec();
    for _ in 0..iters {
        if cur.len() < 3 {
            break;
        }
        let n = cur.len();
        let mut out = Vec::with_capacity(n * 2);
        if !closed {
            out.push(cur[0]);
        }
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (cur[i], cur[(i + 1) % n]);
            out.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            out.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        if !closed {
            out.push(cur[n - 1]);
        }
        cur = out;
    }
    cur
}

// ---------------------------------------------------------------- text metrics

pub fn font() -> &'static ab_glyph::FontRef<'static> {
    static F: OnceLock<ab_glyph::FontRef<'static>> = OnceLock::new();
    F.get_or_init(|| ab_glyph::FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("font"))
}

pub const LINE_HEIGHT: f32 = 1.25;

pub fn line_width(line: &str, size: f32) -> f32 {
    use ab_glyph::{Font, ScaleFont};
    let f = font().as_scaled(size);
    let mut w = 0.0;
    let mut prev = None;
    for ch in line.chars() {
        let g = f.glyph_id(ch);
        if let Some(pg) = prev {
            w += f.kern(pg, g);
        }
        w += f.h_advance(g);
        prev = Some(g);
    }
    w
}

pub fn text_extent(text: &str, size: f32) -> (f32, f32) {
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lines.iter().map(|l| line_width(l, size)).fold(size * 0.3, f32::max);
    (w, lines.len() as f32 * size * LINE_HEIGHT)
}

// ---------------------------------------------------------------- document

impl Layer {
    pub fn new(name: &str, len: usize, color: Rgba) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(0, Keyframe::default());
        Layer { name: name.into(), visible: true, locked: false, outline: false, color, len: len.max(1), keys }
    }

    /// Keyframe index governing `f` (ignores span end).
    pub fn key_at(&self, f: usize) -> usize {
        self.keys.range(..=f).next_back().map(|(k, _)| *k).unwrap_or(0)
    }

    pub fn next_key(&self, k: usize) -> Option<usize> {
        self.keys.range(k + 1..).next().map(|(k, _)| *k)
    }

    /// End (exclusive) of the span that starts at keyframe `k`.
    pub fn span_end(&self, k: usize) -> usize {
        self.next_key(k).unwrap_or(self.len).min(self.len).max(k + 1)
    }

    /// Whether frame `f` is inside a tween (between two keyframes, tweened).
    pub fn tween_at(&self, f: usize) -> Option<(usize, usize, Tween)> {
        if f >= self.len {
            return None;
        }
        let k = self.key_at(f);
        let kf = &self.keys[&k];
        if kf.tween.kind == TweenKind::None {
            return None;
        }
        Some((k, self.span_end(k), kf.tween))
    }

    /// Evaluated shapes at frame `f`.
    pub fn shapes_at(&self, f: usize) -> Vec<Shape> {
        if f >= self.len {
            return Vec::new();
        }
        let k = self.key_at(f);
        let kf = &self.keys[&k];
        if f == k || kf.tween.kind == TweenKind::None {
            return kf.shapes.clone();
        }
        let Some(k2) = self.next_key(k).filter(|k2| *k2 < self.len) else {
            return kf.shapes.clone();
        };
        let t = ease((f - k) as f32 / (k2 - k) as f32, kf.tween.ease);
        let spin = kf.tween.spins as f32 * TAU * t;
        let b = &self.keys[&k2].shapes;
        match kf.tween.kind {
            TweenKind::Classic => kf
                .shapes
                .iter()
                .map(|a| match b.iter().find(|s| s.id == a.id) {
                    Some(bs) => lerp_shape(a, bs, t, spin, false),
                    None => a.clone(),
                })
                .collect(),
            TweenKind::Shape => {
                let mut out = Vec::new();
                for (i, a) in kf.shapes.iter().enumerate() {
                    match b.get(i) {
                        Some(bs) => out.push(lerp_shape(a, bs, t, spin, true)),
                        None => {
                            let mut s = a.clone();
                            fade(&mut s, 1.0 - t);
                            out.push(s);
                        }
                    }
                }
                for bs in b.iter().skip(kf.shapes.len()) {
                    let mut s = bs.clone();
                    fade(&mut s, t);
                    out.push(s);
                }
                out
            }
            TweenKind::None => unreachable!(),
        }
    }
}

fn fade(s: &mut Shape, a: f32) {
    s.for_leaves_mut(&mut |s| {
        for c in [&mut s.fill, &mut s.stroke].into_iter().flatten() {
            c[3] = (c[3] as f32 * a) as u8;
        }
    });
}

pub fn ease(t: f32, e: f32) -> f32 {
    let e = (e / 100.0).clamp(-1.0, 1.0);
    if e >= 0.0 {
        let out = 1.0 - (1.0 - t) * (1.0 - t);
        t + (out - t) * e
    } else {
        t + (t * t - t) * -e
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_col(a: Option<Rgba>, b: Option<Rgba>, t: f32) -> Option<Rgba> {
    match (a, b) {
        (Some(a), Some(b)) => Some(std::array::from_fn(|i| lerp(a[i] as f32, b[i] as f32, t).round() as u8)),
        (Some(mut a), None) => {
            a[3] = (a[3] as f32 * (1.0 - t)) as u8;
            Some(a)
        }
        (None, Some(mut b)) => {
            b[3] = (b[3] as f32 * t) as u8;
            Some(b)
        }
        (None, None) => None,
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > TAU / 2.0 {
        d -= TAU;
    }
    if d < -TAU / 2.0 {
        d += TAU;
    }
    a + d * t
}

fn lerp_shape(a: &Shape, b: &Shape, t: f32, spin: f32, morph: bool) -> Shape {
    let xf = Xform {
        x: lerp(a.xf.x, b.xf.x, t),
        y: lerp(a.xf.y, b.xf.y, t),
        sx: lerp(a.xf.sx, b.xf.sx, t),
        sy: lerp(a.xf.sy, b.xf.sy, t),
        rot: lerp_angle(a.xf.rot, b.xf.rot, t) + spin,
    };
    let mut out = a.clone();
    out.xf = xf;
    out.stroke = lerp_col(a.stroke, b.stroke, t);
    out.fill = lerp_col(a.fill, b.fill, t);
    out.stroke_w = lerp(a.stroke_w, b.stroke_w, t);
    if morph && !a.is_container() && !b.is_container() {
        // Morph in world space then store as an identity-transformed path.
        let (pa, ca) = a.world_outline();
        let (pb, cb) = b.world_outline();
        let closed = ca || cb;
        let n = 128;
        let ra = resample(&pa, closed, n);
        let mut rb = resample(&pb, closed, n);
        if closed {
            align_rotation(&ra, &mut rb);
        }
        let pts = ra.iter().zip(&rb).map(|(p, q)| [lerp(p[0], q[0], t), lerp(p[1], q[1], t)]).collect();
        out.geom = Geom::Path { pts, closed };
        out.xf = Xform::default();
        out.stroke_w = lerp(a.stroke_w * a.xf.avg_scale(), b.stroke_w * b.xf.avg_scale(), t);
    } else if let (Geom::Rect { w, h, radius }, Geom::Rect { w: w2, h: h2, radius: r2 }) = (&a.geom, &b.geom) {
        out.geom = Geom::Rect { w: lerp(*w, *w2, t), h: lerp(*h, *h2, t), radius: lerp(*radius, *r2, t) };
    } else if let (Geom::Ellipse { rx, ry }, Geom::Ellipse { rx: rx2, ry: ry2 }) = (&a.geom, &b.geom) {
        out.geom = Geom::Ellipse { rx: lerp(*rx, *rx2, t), ry: lerp(*ry, *ry2, t) };
    }
    out
}

pub fn resample(pts: &[P], closed: bool, n: usize) -> Vec<P> {
    if pts.is_empty() {
        return vec![[0.0, 0.0]; n];
    }
    let mut seq = pts.to_vec();
    if closed {
        seq.push(pts[0]);
    }
    let mut cum = vec![0.0];
    for w in seq.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        cum.push(cum.last().unwrap() + d);
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return vec![pts[0]; n];
    }
    let denom = if closed { n as f32 } else { (n - 1) as f32 };
    let mut out = Vec::with_capacity(n);
    let mut j = 0;
    for i in 0..n {
        let d = total * i as f32 / denom;
        while j + 1 < cum.len() - 1 && cum[j + 1] < d {
            j += 1;
        }
        let seg = (cum[j + 1] - cum[j]).max(1e-6);
        let u = ((d - cum[j]) / seg).clamp(0.0, 1.0);
        out.push([lerp(seq[j][0], seq[j + 1][0], u), lerp(seq[j][1], seq[j + 1][1], u)]);
    }
    out
}

fn align_rotation(a: &[P], b: &mut Vec<P>) {
    let n = a.len();
    let mut best = (f32::MAX, 0);
    for off in (0..n).step_by(2) {
        let mut d = 0.0;
        for i in (0..n).step_by(4) {
            let q = b[(i + off) % n];
            d += (a[i][0] - q[0]).powi(2) + (a[i][1] - q[1]).powi(2);
        }
        if d < best.0 {
            best = (d, off);
        }
    }
    b.rotate_left(best.1);
}

impl Document {
    pub fn new(width: f32, height: f32, fps: f32) -> Self {
        Document {
            width,
            height,
            fps,
            background: [255, 255, 255, 255],
            layers: vec![Layer::new("Layer_1", 1, LAYER_COLORS[0])],
            next_id: 1,
            script: String::new(),
            on_frame: String::new(),
            on_key: String::new(),
            hub_id: None,
            symbols: Vec::new(),
            functions: String::new(),
            edit_stack: Vec::new(),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.layers.iter().map(|l| l.len).max().unwrap_or(1).max(1)
    }

    pub fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Visible shapes at frame f, back to front, with symbol instances expanded.
    pub fn render_list(&self, f: usize) -> Vec<(usize, Shape)> {
        self.render_list_with(f, &|_, frame| frame)
    }

    /// Like [`render_list`]; `timing(instance, default_frame)` picks each top-level instance's frame
    /// (the player uses it to run movie clips independently).
    pub fn render_list_with(&self, f: usize, timing: &dyn Fn(&Shape, usize) -> usize) -> Vec<(usize, Shape)> {
        let mut out = Vec::new();
        for (li, l) in self.layers.iter().enumerate().rev() {
            if !l.visible {
                continue;
            }
            for s in self.eval_layer(l, f, 0, timing) {
                out.push((li, s));
            }
        }
        out
    }

    /// One layer's shapes at `f` with instances expanded (for hit testing and bounds).
    pub fn layer_shapes(&self, li: usize, f: usize) -> Vec<Shape> {
        self.layers.get(li).map(|l| self.eval_layer(l, f, 0, &|_, frame| frame)).unwrap_or_default()
    }

    fn eval_layer(&self, l: &Layer, f: usize, depth: usize, timing: &dyn Fn(&Shape, usize) -> usize) -> Vec<Shape> {
        let local = f.saturating_sub(l.key_at(f));
        let mut shapes = l.shapes_at(f);
        for s in &mut shapes {
            self.expand(s, local, depth, timing);
        }
        shapes
    }

    fn expand(&self, s: &mut Shape, local: usize, depth: usize, timing: &dyn Fn(&Shape, usize) -> usize) {
        match &mut s.geom {
            Geom::Group { children } => {
                for c in children {
                    self.expand(c, local, depth, timing);
                }
            }
            Geom::Symbol { symbol, first, mode, .. } => {
                let (symbol, first, mode) = (*symbol, *first, *mode);
                let Some(sym) = self.symbol(symbol).filter(|_| depth < 8) else { return };
                let frame = if depth == 0 { timing(s, instance_frame(mode, first, local, sym.frame_count())) } else { instance_frame(mode, first, local, sym.frame_count()) };
                let mut out = Vec::new();
                for l in sym.layers.iter().rev().filter(|l| l.visible) {
                    out.extend(self.eval_layer(l, frame, depth + 1, &|_, f| f));
                }
                if let Geom::Symbol { children, .. } = &mut s.geom {
                    *children = out;
                }
            }
            _ => {}
        }
    }

    /// A symbol's content at one of its frames (for library previews).
    pub fn symbol_shapes(&self, id: u64, frame: usize) -> Vec<Shape> {
        let Some(sym) = self.symbol(id) else { return Vec::new() };
        let mut out = Vec::new();
        for l in sym.layers.iter().rev().filter(|l| l.visible) {
            out.extend(self.eval_layer(l, frame, 1, &|_, f| f));
        }
        out
    }

    pub fn symbol(&self, id: u64) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.id == id)
    }

    pub fn symbol_mut(&mut self, id: u64) -> Option<&mut Symbol> {
        self.symbols.iter_mut().find(|s| s.id == id)
    }

    /// Enters a symbol's timeline for editing (its layers become `self.layers`).
    pub fn enter_symbol(&mut self, ctx: EditCtx) -> bool {
        let Some(i) = self.symbols.iter().position(|s| s.id == ctx.symbol) else { return false };
        std::mem::swap(&mut self.layers, &mut self.symbols[i].layers);
        self.edit_stack.push(ctx);
        true
    }

    pub fn exit_symbol(&mut self) -> Option<EditCtx> {
        let ctx = self.edit_stack.pop()?;
        if let Some(i) = self.symbols.iter().position(|s| s.id == ctx.symbol) {
            std::mem::swap(&mut self.layers, &mut self.symbols[i].layers);
        }
        Some(ctx)
    }

    /// The document with any symbol editing unwound (what gets saved, played and exported).
    pub fn normalized(&self) -> Document {
        let mut d = self.clone();
        while d.exit_symbol().is_some() {}
        d
    }

    /// Whether any timeline (main or a symbol) uses symbol `id`.
    pub fn symbol_in_use(&self, id: u64) -> bool {
        fn uses(s: &Shape, id: u64) -> bool {
            match &s.geom {
                Geom::Symbol { symbol, .. } => *symbol == id,
                Geom::Group { children } => children.iter().any(|c| uses(c, id)),
                _ => false,
            }
        }
        let layers = self.layers.iter().chain(self.symbols.iter().flat_map(|s| s.layers.iter()));
        layers.flat_map(|l| l.keys.values()).any(|k| k.shapes.iter().any(|s| uses(s, id)))
    }

    /// Returns a keyframe that edits at frame `f` should modify, creating one if needed
    /// (on tweened in-betweens or past the end of the layer).
    pub fn edit_key(&mut self, li: usize, f: usize) -> &mut Keyframe {
        let l = &mut self.layers[li];
        if f >= l.len {
            insert_keyframe(l, f, true);
        } else if !l.keys.contains_key(&f) && l.tween_at(f).is_some() {
            insert_keyframe(l, f, false);
        }
        let k = l.key_at(f);
        l.keys.get_mut(&k).unwrap()
    }

    pub fn shape_mut(&mut self, li: usize, f: usize, id: u64) -> Option<&mut Shape> {
        self.edit_key(li, f).shapes.iter_mut().find(|s| s.id == id)
    }
}

pub const LAYER_COLORS: [Rgba; 8] = [
    [74, 144, 255, 255],
    [255, 92, 92, 255],
    [80, 220, 120, 255],
    [255, 200, 60, 255],
    [190, 110, 255, 255],
    [60, 220, 220, 255],
    [255, 140, 220, 255],
    [255, 150, 60, 255],
];

// ---------------------------------------------------------------- timeline ops

pub fn insert_frame(l: &mut Layer, f: usize) {
    if f >= l.len {
        l.len = f + 1;
        return;
    }
    shift_keys(l, f + 1, 1);
    l.len += 1;
}

pub fn remove_frame(l: &mut Layer, f: usize) {
    if f >= l.len || l.len <= 1 {
        return;
    }
    let k = l.key_at(f);
    if l.span_end(k) - k == 1 && k != 0 {
        l.keys.remove(&k);
    }
    let moved: Vec<(usize, Keyframe)> = {
        let ks: Vec<usize> = l.keys.range(f + 1..).map(|(k, _)| *k).collect();
        ks.into_iter().map(|k| (k, l.keys.remove(&k).unwrap())).collect()
    };
    for (k, kf) in moved {
        l.keys.entry(k - 1).or_insert(kf);
    }
    l.len -= 1;
}

fn shift_keys(l: &mut Layer, from: usize, by: usize) {
    let ks: Vec<usize> = l.keys.range(from..).map(|(k, _)| *k).rev().collect();
    for k in ks {
        let kf = l.keys.remove(&k).unwrap();
        l.keys.insert(k + by, kf);
    }
}

pub fn insert_keyframe(l: &mut Layer, f: usize, blank: bool) {
    if l.keys.contains_key(&f) && f < l.len {
        return;
    }
    let prev = l.key_at(f);
    let mut shapes = if blank { Vec::new() } else { l.shapes_at(f.min(l.len - 1).max(prev)) };
    if !blank && f >= l.len {
        shapes = l.keys[&prev].shapes.clone();
    }
    let tween = if f < l.len { l.keys[&prev].tween } else { Tween::default() };
    if f >= l.len {
        l.len = f + 1;
    }
    l.keys.insert(f, Keyframe { shapes, tween, ..Default::default() });
}

pub fn clear_keyframe(l: &mut Layer, f: usize) {
    if !l.keys.contains_key(&f) {
        return;
    }
    if f == 0 {
        l.keys.insert(0, Keyframe::default());
    } else {
        l.keys.remove(&f);
    }
}
