//! The startup intro (assets/intro.flx), played full-window when Flint and Flint Player open.

use crate::render::{self, View, c32};
use crate::runtime::Runtime;
use crate::flx;
use egui::{Color32, Rect, Ui, Vec2};
use std::time::Instant;

pub const INTRO_FLX: &[u8] = include_bytes!("../assets/intro.flx");

pub struct Intro {
    rt: Runtime,
    start: Instant,
    secs: f64,
}

impl Intro {
    /// None when disabled with `--no-intro` (or if the embedded movie can't be read).
    pub fn new() -> Option<Self> {
        if std::env::args().any(|a| a == "--no-intro") {
            return None;
        }
        let doc = flx::decode(INTRO_FLX).ok()?;
        let secs = doc.frame_count() as f64 / doc.fps.max(1.0) as f64;
        let mut rt = Runtime::new(doc);
        rt.set_looping(false);
        Some(Intro { rt, start: Instant::now(), secs })
    }

    /// Plays one frame over the whole window. Returns true once it has finished or been skipped.
    pub fn show(&mut self, ui: &mut Ui) -> bool {
        let ctx = ui.ctx().clone();
        let skipped = ctx.input(|i| i.pointer.any_click() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if skipped || self.start.elapsed().as_secs_f64() > self.secs + 0.4 {
            return true;
        }
        let rect = ui.max_rect();
        let (w, h, bg) = {
            let d = self.rt.doc();
            (d.width, d.height, d.background)
        };
        let zoom = (rect.width() / w).min(rect.height() / h).max(0.01);
        let size = Vec2::new(w, h) * zoom;
        let view = View { origin: rect.center() - size * 0.5, zoom };
        self.rt.tick(ctx.input(|i| i.stable_dt).min(0.1) as f64);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, c32(bg));
        let stage = Rect::from_min_size(view.origin, size);
        let clipped = painter.with_clip_rect(stage);
        let style = render::Style { alpha: 1.0, outline: None };
        for (_, s) in self.rt.shapes() {
            render::draw_shape(&clipped, &view, &s, &style);
        }
        let hint = if bg[0] as u32 + bg[1] as u32 + bg[2] as u32 > 380 { Color32::from_gray(150) } else { Color32::from_gray(110) };
        painter.text(rect.right_bottom() - Vec2::new(14.0, 10.0), egui::Align2::RIGHT_BOTTOM, "click or press any key to skip", egui::FontId::proportional(12.0), hint);
        ctx.request_repaint();
        false
    }
}
