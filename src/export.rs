//! Rasterizing / exporting frames: PNG sequence, animated GIF, SVG.

use crate::model::*;
use std::path::Path;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn poly_path(pts: &[P], closed: bool) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(pts.first()?[0], pts[0][1]);
    if pts.len() == 1 {
        pb.line_to(pts[0][0] + 0.01, pts[0][1]);
    }
    for p in &pts[1..] {
        pb.line_to(p[0], p[1]);
    }
    if closed {
        pb.close();
    }
    pb.finish()
}

/// Glyph outlines of a text shape, in world space.
pub fn text_path(s: &Shape) -> Option<tiny_skia::Path> {
    use ab_glyph::{Font, OutlineCurve, ScaleFont};
    let Geom::Text { text, size } = &s.geom else { return None };
    let f = font();
    let sf = f.as_scaled(*size);
    let k = sf.h_scale_factor();
    let mut pb = PathBuilder::new();
    for (li, line) in text.split('\n').enumerate() {
        let baseline = li as f32 * size * LINE_HEIGHT + sf.ascent();
        let mut pen = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let g = f.glyph_id(ch);
            if let Some(pg) = prev {
                pen += sf.kern(pg, g);
            }
            if let Some(o) = f.outline(g) {
                let map = |p: ab_glyph::Point| s.xf.apply([pen + p.x * k, baseline - p.y * k]);
                let mut last: Option<ab_glyph::Point> = None;
                for c in &o.curves {
                    let start = match c {
                        OutlineCurve::Line(a, _) | OutlineCurve::Quad(a, _, _) | OutlineCurve::Cubic(a, _, _, _) => *a,
                    };
                    if last.is_none_or(|l| l != start) {
                        if last.is_some() {
                            pb.close();
                        }
                        let q = map(start);
                        pb.move_to(q[0], q[1]);
                    }
                    match c {
                        OutlineCurve::Line(_, b) => {
                            let q = map(*b);
                            pb.line_to(q[0], q[1]);
                            last = Some(*b);
                        }
                        OutlineCurve::Quad(_, c1, b) => {
                            let (q1, q) = (map(*c1), map(*b));
                            pb.quad_to(q1[0], q1[1], q[0], q[1]);
                            last = Some(*b);
                        }
                        OutlineCurve::Cubic(_, c1, c2, b) => {
                            let (q1, q2, q) = (map(*c1), map(*c2), map(*b));
                            pb.cubic_to(q1[0], q1[1], q2[0], q2[1], q[0], q[1]);
                            last = Some(*b);
                        }
                    }
                }
                if last.is_some() {
                    pb.close();
                }
            }
            pen += sf.h_advance(g);
            prev = Some(g);
        }
    }
    pb.finish()
}

pub fn render_frame(doc: &Document, f: usize, scale: f32) -> Pixmap {
    let w = (doc.width * scale).round().max(1.0) as u32;
    let h = (doc.height * scale).round().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    let bg = doc.background;
    pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    let tf = Transform::from_scale(scale, scale);
    for s in doc.render_list(f).into_iter().flat_map(|(_, s)| s.flatten()) {
        if s.is_text() {
            if let (Some(fill), Some(path)) = (s.fill, text_path(&s)) {
                pm.fill_path(&path, &paint(fill), FillRule::Winding, tf, None);
            }
            continue;
        }
        let (pts, closed) = s.world_outline();
        let Some(path) = poly_path(&pts, closed) else { continue };
        if closed {
            if let Some(fill) = s.fill {
                pm.fill_path(&path, &paint(fill), FillRule::EvenOdd, tf, None);
            }
        }
        if let Some(sc) = s.stroke {
            let st = Stroke {
                width: (s.stroke_w * s.xf.avg_scale()).max(0.1),
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            pm.stroke_path(&path, &paint(sc), &st, tf, None);
        }
    }
    pm
}

pub fn export_png_sequence(doc: &Document, dir: &Path, base: &str) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let n = doc.frame_count();
    for f in 0..n {
        let pm = render_frame(doc, f, 1.0);
        pm.save_png(dir.join(format!("{base}_{:04}.png", f + 1))).map_err(|e| e.to_string())?;
    }
    Ok(n)
}

pub fn export_png(doc: &Document, f: usize, path: &Path) -> Result<(), String> {
    render_frame(doc, f, 1.0).save_png(path).map_err(|e| e.to_string())
}

pub fn export_gif(doc: &Document, path: &Path) -> Result<usize, String> {
    let n = doc.frame_count();
    let w = doc.width.round().clamp(1.0, 65535.0) as u16;
    let h = doc.height.round().clamp(1.0, 65535.0) as u16;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = gif::Encoder::new(file, w, h, &[]).map_err(|e| e.to_string())?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
    let delay = (100.0 / doc.fps.max(1.0)).round().max(2.0) as u16;
    for f in 0..n {
        let pm = render_frame(doc, f, 1.0);
        let mut px: Vec<u8> = pm.pixels().iter().flat_map(|c| {
            let c = c.demultiply();
            [c.red(), c.green(), c.blue(), 255]
        }).collect();
        let mut fr = gif::Frame::from_rgba_speed(w, h, &mut px, 10);
        fr.delay = delay;
        enc.write_frame(&fr).map_err(|e| e.to_string())?;
    }
    Ok(n)
}

fn svg_col(c: Rgba) -> String {
    format!("rgb({},{},{})\" {}-opacity=\"{:.3}", c[0], c[1], c[2], "{K}", c[3] as f32 / 255.0)
}

pub fn export_svg(doc: &Document, f: usize, path: &Path) -> Result<(), String> {
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>\n",
        svg_col(doc.background).replace("{K}", "fill"),
        w = doc.width,
        h = doc.height
    );
    for sh in doc.render_list(f).into_iter().flat_map(|(_, s)| s.flatten()) {
        if let Geom::Text { text, size } = &sh.geom {
            let Some(fill) = sh.fill else { continue };
            let deg = sh.xf.rot.to_degrees();
            for (i, line) in text.split('\n').enumerate() {
                let esc = line.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                s += &format!(
                    "<text transform=\"translate({} {}) rotate({deg}) scale({} {})\" x=\"0\" y=\"{}\" font-family=\"Ubuntu, sans-serif\" font-weight=\"300\" font-size=\"{size}\" fill=\"{}\">{esc}</text>\n",
                    sh.xf.x, sh.xf.y, sh.xf.sx, sh.xf.sy,
                    i as f32 * size * LINE_HEIGHT + size * 0.95,
                    svg_col(fill).replace("{K}", "fill")
                );
            }
            continue;
        }
        let (pts, closed) = sh.world_outline();
        if pts.is_empty() {
            continue;
        }
        let mut d = String::new();
        for (i, p) in pts.iter().enumerate() {
            d += &format!("{}{:.2},{:.2} ", if i == 0 { "M" } else { "L" }, p[0], p[1]);
        }
        if closed {
            d += "Z";
        }
        let fill = match (closed, sh.fill) {
            (true, Some(c)) => svg_col(c).replace("{K}", "fill"),
            _ => "none".into(),
        };
        let stroke = match sh.stroke {
            Some(c) => format!(
                "{}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" stroke-linejoin=\"round",
                svg_col(c).replace("{K}", "stroke"),
                sh.stroke_w * sh.xf.avg_scale()
            ),
            None => "none".into(),
        };
        s += &format!("<path d=\"{d}\" fill=\"{fill}\" fill-rule=\"evenodd\" stroke=\"{stroke}\"/>\n");
    }
    s += "</svg>\n";
    std::fs::write(path, s).map_err(|e| e.to_string())
}
