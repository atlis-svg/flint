//! The editor: menus, tools panel, stage, timeline and properties.

use crate::export;
use crate::{flx, hub, runtime};
use crate::model::*;
use crate::render::{self, View, c32, rgba};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Event, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Transform,
    Pencil,
    Brush,
    Line,
    Rect,
    Oval,
    Bucket,
    Ink,
    Dropper,
    Eraser,
    Text,
    Hand,
    Zoom,
}

const TOOLS: &[(Tool, &str, &str, Key)] = &[
    (Tool::Select, "⬉", "Selection Tool (V)", Key::V),
    (Tool::Transform, "⛶", "Free Transform Tool (Q)", Key::Q),
    (Tool::Pencil, "✏", "Pencil Tool (Y)", Key::Y),
    (Tool::Brush, "🖌", "Brush Tool (B)", Key::B),
    (Tool::Line, "╱", "Line Tool (N)", Key::N),
    (Tool::Rect, "⬜", "Rectangle Tool (R)", Key::R),
    (Tool::Oval, "⭕", "Oval Tool (O)", Key::O),
    (Tool::Bucket, "🎨", "Paint Bucket Tool (K)", Key::K),
    (Tool::Ink, "🖊", "Ink Bottle Tool (S)", Key::S),
    (Tool::Dropper, "💧", "Eyedropper Tool (I)", Key::I),
    (Tool::Eraser, "⌫", "Eraser Tool (E)", Key::E),
    (Tool::Text, "T", "Text Tool (T)", Key::T),
    (Tool::Hand, "✋", "Hand Tool (H)", Key::H),
    (Tool::Zoom, "🔍", "Zoom Tool (Z)", Key::Z),
];

#[derive(Clone)]
enum Drag {
    None,
    Draw(Vec<P>),
    Box(P, P),
    Marquee(P, P),
    Move { start: P, orig: Vec<(usize, u64, Xform)>, corner: P },
    Scale { handle: usize, pivot: P, start: P, orig: Vec<(usize, u64, Xform)> },
    Rotate { pivot: P, start: P, orig: Vec<(usize, u64, Xform)> },
    Erase,
    Pan,
    Scrub,
}

pub struct FlintApp {
    doc: Document,
    path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<Document>,
    redo: Vec<Document>,
    frame: usize,
    layer: usize,
    playing: bool,
    play_accum: f64,
    looping: bool,
    tool: Tool,
    stroke_col: Color32,
    fill_col: Color32,
    stroke_on: bool,
    fill_on: bool,
    stroke_w: f32,
    brush_size: f32,
    smoothing: f32,
    rect_radius: f32,
    font_size: f32,
    eraser_size: f32,
    bucket_gap: f32,
    sel: Vec<(usize, u64)>,
    zoom: f32,
    pan: Vec2,
    fit_pending: bool,
    drag: Drag,
    onion: bool,
    onion_range: usize,
    clip_stage: bool,
    show_grid: bool,
    snap_grid: bool,
    grid_size: f32,
    text_edit: Option<(usize, u64)>,
    text_focus: bool,
    status: String,
    renaming: Option<usize>,
    ctx_cell: (usize, usize),
    new_dialog: Option<(f32, f32, f32)>,
    show_actions: bool,
    actions_target: ActTarget,
    script_errors: Vec<(String, String)>,
    script_active: bool,
    hub: hub::Hub,
    publish: Option<PublishDialog>,
    intro: Option<crate::intro::Intro>,
    right_tab: u8,
    show_about: bool,
    lib_sel: Option<u64>,
    canvas_center: Pos2,
    prop_active: bool,
    last_title: String,
    view: View,
}

impl FlintApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let mut vis = egui::Visuals::dark();
        vis.panel_fill = Color32::from_gray(40);
        vis.window_fill = Color32::from_gray(45);
        vis.extreme_bg_color = Color32::from_gray(28);
        vis.selection.bg_fill = Color32::from_rgb(38, 110, 210);
        cc.egui_ctx.set_visuals(vis);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = Self::blank(Document::new(1280.0, 720.0, 24.0));
        app.intro = crate::intro::Intro::new();
        if let Some(p) = std::env::args().nth(1).filter(|a| !a.starts_with("--")) {
            app.open_path(PathBuf::from(p));
        }
        app
    }

    fn blank(doc: Document) -> Self {
        FlintApp {
            doc,
            path: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            frame: 0,
            layer: 0,
            playing: false,
            play_accum: 0.0,
            looping: true,
            tool: Tool::Select,
            stroke_col: Color32::BLACK,
            fill_col: Color32::from_rgb(0, 102, 255),
            stroke_on: true,
            fill_on: true,
            stroke_w: 2.0,
            brush_size: 8.0,
            smoothing: 50.0,
            rect_radius: 0.0,
            font_size: 36.0,
            eraser_size: 16.0,
            bucket_gap: 3.0,
            sel: Vec::new(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_pending: true,
            drag: Drag::None,
            onion: false,
            onion_range: 2,
            clip_stage: false,
            show_grid: false,
            snap_grid: false,
            grid_size: 20.0,
            text_edit: None,
            text_focus: false,
            status: "Ready".into(),
            renaming: None,
            ctx_cell: (0, 0),
            new_dialog: None,
            show_actions: false,
            actions_target: ActTarget::Frame,
            script_errors: Vec::new(),
            script_active: false,
            hub: hub::Hub::load(),
            publish: None,
            intro: None,
            right_tab: 0,
            show_about: false,
            lib_sel: None,
            canvas_center: Pos2::ZERO,
            prop_active: false,
            last_title: String::new(),
            view: View { origin: Pos2::ZERO, zoom: 1.0 },
        }
    }

    // ------------------------------------------------------------ undo / doc helpers

    fn checkpoint(&mut self) {
        self.undo.push(self.doc.clone());
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn do_undo(&mut self) {
        self.finish_text();
        if let Some(d) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.after_doc_swap();
            self.status = "Undo".into();
        }
    }

    fn do_redo(&mut self) {
        if let Some(d) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, d));
            self.after_doc_swap();
            self.status = "Redo".into();
        }
    }

    fn after_doc_swap(&mut self) {
        self.dirty = true;
        self.layer = self.layer.min(self.doc.layers.len() - 1);
        self.prune_sel();
    }

    fn prune_sel(&mut self) {
        let f = self.frame;
        let doc = &self.doc;
        self.sel.retain(|(li, id)| *li < doc.layers.len() && doc.layers[*li].shapes_at(f).iter().any(|s| s.id == *id));
    }

    fn set_frame(&mut self, f: usize) {
        if f != self.frame {
            self.finish_text();
            self.frame = f;
            self.prune_sel();
        }
    }

    fn sel_shapes(&self) -> Vec<(usize, Shape)> {
        let mut out = Vec::new();
        for (li, id) in &self.sel {
            if let Some(s) = self.doc.layer_shapes(*li, self.frame).into_iter().find(|s| s.id == *id) {
                out.push((*li, s));
            }
        }
        out
    }

    fn sel_bounds(&self) -> Option<[f32; 4]> {
        self.sel_shapes().iter().map(|(_, s)| s.bounds()).reduce(union_bounds)
    }

    fn layer_editable(&self, li: usize) -> bool {
        self.doc.layers.get(li).is_some_and(|l| l.visible && !l.locked)
    }

    fn add_shape(&mut self, mut s: Shape) {
        if !self.layer_editable(self.layer) {
            self.status = "Active layer is locked or hidden".into();
            return;
        }
        self.checkpoint();
        s.id = self.doc.alloc_id();
        let id = s.id;
        self.doc.edit_key(self.layer, self.frame).shapes.push(s);
        self.sel = vec![(self.layer, id)];
    }

    fn hit_test(&self, p: P, only_layer: Option<usize>) -> Option<(usize, Shape)> {
        let tol = 4.0 / self.zoom;
        for (li, l) in self.doc.layers.iter().enumerate() {
            if !l.visible || l.locked || only_layer.is_some_and(|o| o != li) {
                continue;
            }
            for s in self.doc.layer_shapes(li, self.frame).into_iter().rev() {
                if s.hit(p, tol) {
                    return Some((li, s));
                }
            }
        }
        None
    }

    fn cur_stroke(&self) -> Option<Rgba> {
        self.stroke_on.then(|| rgba(self.stroke_col))
    }
    fn cur_fill(&self) -> Option<Rgba> {
        self.fill_on.then(|| rgba(self.fill_col))
    }

    fn for_selected(&mut self, mut f: impl FnMut(&mut Shape)) {
        let frame = self.frame;
        for (li, id) in self.sel.clone() {
            if let Some(s) = self.doc.shape_mut(li, frame, id) {
                f(s);
            }
        }
    }

    fn set_xforms(&mut self, xs: &[(usize, u64, Xform)]) {
        let frame = self.frame;
        for (li, id, xf) in xs {
            if let Some(s) = self.doc.shape_mut(*li, frame, *id) {
                s.xf = *xf;
            }
        }
    }

    fn sel_xforms(&self) -> Vec<(usize, u64, Xform)> {
        self.sel_shapes().into_iter().map(|(li, s)| (li, s.id, s.xf)).collect()
    }

    // ------------------------------------------------------------ edit commands

    fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let frame = self.frame;
        for (li, id) in std::mem::take(&mut self.sel) {
            self.doc.edit_key(li, frame).shapes.retain(|s| s.id != id);
        }
    }

    fn copy_selection(&mut self, ctx: &egui::Context) {
        let shapes: Vec<Shape> = self.sel_shapes().into_iter().map(|(_, s)| s).collect();
        if shapes.is_empty() {
            return;
        }
        if let Ok(j) = serde_json::to_string(&shapes) {
            ctx.copy_text(format!("FLINT_SHAPES:{j}"));
            self.status = format!("Copied {} object(s)", shapes.len());
        }
    }

    fn paste(&mut self, text: &str, in_place: bool) {
        let Some(j) = text.strip_prefix("FLINT_SHAPES:") else { return };
        let Ok(shapes) = serde_json::from_str::<Vec<Shape>>(j) else { return };
        if !self.layer_editable(self.layer) {
            return;
        }
        self.checkpoint();
        self.sel.clear();
        let frame = self.frame;
        for mut s in shapes {
            s.id = self.doc.alloc_id();
            if !in_place {
                s.xf.x += 10.0;
                s.xf.y += 10.0;
            }
            self.sel.push((self.layer, s.id));
            self.doc.edit_key(self.layer, frame).shapes.push(s);
        }
    }

    fn duplicate(&mut self) {
        let shapes = self.sel_shapes();
        if shapes.is_empty() {
            return;
        }
        self.checkpoint();
        self.sel.clear();
        let frame = self.frame;
        for (li, mut s) in shapes {
            s.id = self.doc.alloc_id();
            s.xf.x += 10.0;
            s.xf.y += 10.0;
            self.sel.push((li, s.id));
            self.doc.edit_key(li, frame).shapes.push(s);
        }
    }

    fn select_all(&mut self) {
        self.sel.clear();
        for (li, l) in self.doc.layers.iter().enumerate() {
            if l.visible && !l.locked {
                for s in l.shapes_at(self.frame) {
                    self.sel.push((li, s.id));
                }
            }
        }
    }

    /// to: 0 = back, 1 = backward, 2 = forward, 3 = front
    fn arrange(&mut self, to: u8) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let frame = self.frame;
        for (li, id) in self.sel.clone() {
            let v = &mut self.doc.edit_key(li, frame).shapes;
            let Some(i) = v.iter().position(|s| s.id == id) else { continue };
            let s = v.remove(i);
            let j = match to {
                0 => 0,
                1 => i.saturating_sub(1),
                2 => (i + 1).min(v.len()),
                _ => v.len(),
            };
            v.insert(j, s);
        }
    }

    fn flip(&mut self, horizontal: bool) {
        let Some(b) = self.sel_bounds() else { return };
        self.checkpoint();
        let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
        self.for_selected(|s| {
            if horizontal {
                s.xf.x = 2.0 * cx - s.xf.x;
                s.xf.sx = -s.xf.sx;
                s.xf.rot = -s.xf.rot;
            } else {
                s.xf.y = 2.0 * cy - s.xf.y;
                s.xf.sy = -s.xf.sy;
                s.xf.rot = -s.xf.rot;
            }
        });
    }

    /// mode 0..6: left, h-center, right, top, v-center, bottom
    fn align(&mut self, mode: u8, to_stage: bool) {
        let shapes = self.sel_shapes();
        if shapes.is_empty() {
            return;
        }
        let target = if to_stage {
            [0.0, 0.0, self.doc.width, self.doc.height]
        } else {
            self.sel_bounds().unwrap()
        };
        self.checkpoint();
        let frame = self.frame;
        for (li, s) in shapes {
            let b = s.bounds();
            let (dx, dy) = match mode {
                0 => (target[0] - b[0], 0.0),
                1 => ((target[0] + target[2] - b[0] - b[2]) * 0.5, 0.0),
                2 => (target[2] - b[2], 0.0),
                3 => (0.0, target[1] - b[1]),
                4 => (0.0, (target[1] + target[3] - b[1] - b[3]) * 0.5),
                _ => (0.0, target[3] - b[3]),
            };
            if let Some(m) = self.doc.shape_mut(li, frame, s.id) {
                m.xf.x += dx;
                m.xf.y += dy;
            }
        }
    }

    fn nudge(&mut self, dx: f32, dy: f32) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        self.for_selected(|s| {
            s.xf.x += dx;
            s.xf.y += dy;
        });
    }

    // ------------------------------------------------------------ timeline commands

    fn tl_insert_frame(&mut self) {
        self.checkpoint();
        let f = self.frame;
        insert_frame(&mut self.doc.layers[self.layer], f);
    }
    fn tl_remove_frame(&mut self) {
        self.checkpoint();
        let f = self.frame;
        remove_frame(&mut self.doc.layers[self.layer], f);
        self.prune_sel();
    }
    fn tl_keyframe(&mut self, blank: bool) {
        self.checkpoint();
        let f = self.frame;
        insert_keyframe(&mut self.doc.layers[self.layer], f, blank);
        self.prune_sel();
    }
    fn tl_clear_keyframe(&mut self) {
        self.checkpoint();
        let f = self.frame;
        clear_keyframe(&mut self.doc.layers[self.layer], f);
        self.prune_sel();
    }
    fn tl_set_tween(&mut self, kind: TweenKind) {
        let l = &self.doc.layers[self.layer];
        if self.frame >= l.len {
            return;
        }
        let k = l.key_at(self.frame);
        self.checkpoint();
        let l = &mut self.doc.layers[self.layer];
        if kind != TweenKind::None && l.next_key(k).is_none_or(|n| n >= l.len) {
            // like Animate: tweening a span with no end keyframe adds one at the span end
            let end = l.len - 1;
            if end > k {
                insert_keyframe(l, end, false);
            }
        }
        l.keys.get_mut(&k).unwrap().tween.kind = kind;
        self.status = format!("{kind:?} tween");
    }

    fn add_layer(&mut self) {
        self.checkpoint();
        let n = self.doc.layers.len();
        let mut i = n + 1;
        while self.doc.layers.iter().any(|l| l.name == format!("Layer_{i}")) {
            i += 1;
        }
        let l = Layer::new(&format!("Layer_{i}"), self.doc.frame_count(), LAYER_COLORS[n % LAYER_COLORS.len()]);
        self.doc.layers.insert(self.layer, l);
        self.sel.clear();
    }

    fn delete_layer(&mut self) {
        if self.doc.layers.len() <= 1 {
            return;
        }
        self.checkpoint();
        self.doc.layers.remove(self.layer);
        self.layer = self.layer.min(self.doc.layers.len() - 1);
        self.sel.clear();
    }

    fn move_layer(&mut self, up: bool) {
        let li = self.layer;
        let to = if up { li.checked_sub(1) } else { Some(li + 1).filter(|t| *t < self.doc.layers.len()) };
        if let Some(to) = to {
            self.checkpoint();
            self.doc.layers.swap(li, to);
            self.layer = to;
            self.sel.clear();
        }
    }

    // ------------------------------------------------------------ files

    fn file_new(&mut self) {
        self.new_dialog = Some((self.doc.width, self.doc.height, self.doc.fps));
    }

    fn file_open(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Flint document or movie", &["flint", "flx"]).pick_file() {
            self.open_path(p);
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match flx::load(&p) {
            Ok(doc) => {
                let (tool, sc, fc) = (self.tool, self.stroke_col, self.fill_col);
                *self = Self::blank(doc);
                self.tool = tool;
                self.stroke_col = sc;
                self.fill_col = fc;
                self.status = format!("Opened {}", p.display());
                // a published .flx opens as a new, unsaved document
                self.path = (p.extension().and_then(|e| e.to_str()) != Some("flx")).then_some(p);
                self.dirty = self.path.is_none();
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    fn file_save(&mut self, save_as: bool) {
        let path = match (&self.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("Flint document", &["flint"]).set_file_name("untitled.flint").save_file(),
        };
        let Some(mut p) = path else { return };
        if p.extension().is_none() {
            p.set_extension("flint");
        }
        match serde_json::to_string(&self.doc.normalized()).map_err(|e| e.to_string()).and_then(|s| std::fs::write(&p, s).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", p.display());
                self.path = Some(p);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn base_name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "untitled".into())
    }

    fn export(&mut self, kind: &str) {
        self.finish_text();
        let base = self.base_name();
        let doc = self.doc.normalized();
        let main_frame = self.main_frame();
        let res = match kind {
            "png_seq" => rfd::FileDialog::new().pick_folder().map(|d| export::export_png_sequence(&doc, &d, &base).map(|n| format!("Exported {n} PNG frames to {}", d.display()))),
            "png" => rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{base}.png")).save_file().map(|p| export::export_png(&doc, main_frame, &p).map(|_| format!("Exported {}", p.display()))),
            "gif" => rfd::FileDialog::new().add_filter("GIF", &["gif"]).set_file_name(format!("{base}.gif")).save_file().map(|p| export::export_gif(&doc, &p).map(|n| format!("Exported {n}-frame GIF {}", p.display()))),
            "flx" => rfd::FileDialog::new().add_filter("Flint movie", &["flx"]).set_file_name(format!("{base}.flx")).save_file().map(|p| {
                flx::save(&doc, &p).map(|n| format!("Published {} ({:.1} KB) — open it with flint-player", p.display(), n as f32 / 1024.0))
            }),
            "projector" => rfd::FileDialog::new().set_file_name(base.clone()).save_file().map(|p| match flx::player_exe() {
                Some(player) => flx::write_projector(&player, &doc, &p).map(|_| format!("Projector written to {} — runs without Flint installed", p.display())),
                None => Err("flint-player not found next to flint; build it with `cargo build --release`".into()),
            }),
            _ => rfd::FileDialog::new().add_filter("SVG", &["svg"]).set_file_name(format!("{base}.svg")).save_file().map(|p| export::export_svg(&doc, main_frame, &p).map(|_| format!("Exported {}", p.display()))),
        };
        if let Some(r) = res {
            self.status = r.unwrap_or_else(|e| format!("Export failed: {e}"));
        }
        if matches!(kind, "flx" | "projector") {
            self.check_scripts();
        }
    }

    // ------------------------------------------------------------ text editing

    fn finish_text(&mut self) {
        if let Some((li, id)) = self.text_edit.take() {
            let frame = self.frame;
            let k = &mut self.doc.edit_key(li, frame).shapes;
            k.retain(|s| !(s.id == id && matches!(&s.geom, Geom::Text { text, .. } if text.trim().is_empty())));
        }
    }

    // ------------------------------------------------------------ main ui

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for ev in &events {
            match ev {
                Event::Copy if !typing => self.copy_selection(ctx),
                Event::Cut if !typing => {
                    self.copy_selection(ctx);
                    self.delete_selection();
                }
                Event::Paste(t) if !typing => {
                    let in_place = ctx.input(|i| i.modifiers.shift);
                    self.paste(t, in_place);
                }
                _ => {}
            }
        }
        let cmd = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, k));
        let cmd_shift = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, k));
        if cmd_shift(Key::Z) || cmd(Key::Y) {
            self.do_redo();
        }
        if cmd(Key::Z) {
            self.do_undo();
        }
        if cmd_shift(Key::P) {
            self.open_publish();
        }
        if cmd_shift(Key::S) {
            self.file_save(true);
        }
        if cmd(Key::S) {
            self.file_save(false);
        }
        if cmd(Key::O) {
            self.file_open();
        }
        if cmd(Key::N) {
            self.file_new();
        }
        if cmd(Key::Equals) || cmd(Key::Plus) {
            self.zoom_by(2.0, None);
        }
        if cmd(Key::Minus) {
            self.zoom_by(0.5, None);
        }
        if cmd(Key::Num0) {
            self.fit_pending = true;
        }
        if cmd(Key::Num1) {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        if typing {
            return;
        }
        if cmd(Key::Enter) {
            self.test_movie();
        }
        if cmd_shift(Key::ArrowUp) {
            self.arrange(3);
        }
        if cmd_shift(Key::ArrowDown) {
            self.arrange(0);
        }
        if cmd(Key::ArrowUp) {
            self.arrange(2);
        }
        if cmd(Key::ArrowDown) {
            self.arrange(1);
        }
        if cmd(Key::A) {
            self.select_all();
        }
        if cmd_shift(Key::G) || cmd_shift(Key::B) {
            self.ungroup();
        }
        if cmd(Key::F8) {
            self.new_symbol();
        }
        if cmd(Key::E) {
            self.exit_symbol();
        }
        if cmd(Key::G) {
            self.group();
        }
        if cmd(Key::B) {
            self.ungroup();
        }
        if cmd_shift(Key::D) {
            self.distribute_to_layers();
        }
        if cmd_shift(Key::Quote) {
            self.snap_grid = !self.snap_grid;
        }
        if cmd(Key::Quote) {
            self.show_grid = !self.show_grid;
        }
        if cmd(Key::D) {
            self.duplicate();
        }
        let shift_key = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, k));
        if shift_key(Key::F5) {
            self.tl_remove_frame();
        }
        if shift_key(Key::F6) {
            self.tl_clear_keyframe();
        }
        let big = ctx.input(|i| i.modifiers.shift);
        let step = if big { 10.0 } else { 1.0 };
        let key = |k: Key| ctx.input(|i| i.key_pressed(k) && !i.modifiers.command && !i.modifiers.alt);
        if key(Key::ArrowLeft) {
            self.nudge(-step, 0.0);
        }
        if key(Key::ArrowRight) {
            self.nudge(step, 0.0);
        }
        if key(Key::ArrowUp) {
            self.nudge(0.0, -step);
        }
        if key(Key::ArrowDown) {
            self.nudge(0.0, step);
        }
        let plain = |k: Key| ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        if plain(Key::F5) {
            self.tl_insert_frame();
        }
        if plain(Key::F6) {
            self.tl_keyframe(false);
        }
        if plain(Key::F8) {
            self.convert_to_symbol();
        }
        if plain(Key::F9) {
            self.show_actions = !self.show_actions;
        }
        if plain(Key::F7) {
            self.tl_keyframe(true);
        }
        if plain(Key::Enter) {
            self.playing = !self.playing;
        }
        if plain(Key::Comma) {
            self.playing = false;
            self.set_frame(self.frame.saturating_sub(1));
        }
        if plain(Key::Period) {
            self.playing = false;
            self.set_frame(self.frame + 1);
        }
        if plain(Key::Home) {
            self.set_frame(0);
        }
        if plain(Key::End) {
            self.set_frame(self.doc.frame_count() - 1);
        }
        if plain(Key::Delete) || plain(Key::Backspace) {
            self.delete_selection();
        }
        if plain(Key::Escape) {
            self.sel.clear();
        }
        for (t, _, _, k) in TOOLS {
            if plain(*k) {
                self.set_tool(*t);
            }
        }
    }

    fn set_tool(&mut self, t: Tool) {
        if t != self.tool {
            self.finish_text();
            self.tool = t;
        }
    }

    fn zoom_by(&mut self, factor: f32, about: Option<Pos2>) {
        let new_zoom = (self.zoom * factor).clamp(0.05, 32.0);
        let about = about.unwrap_or(self.view.origin + Vec2::new(self.doc.width, self.doc.height) * self.zoom * 0.5);
        let sp = self.view.to_stage(about);
        // keep `sp` under `about`
        let new_origin = about - render::v2(sp) * new_zoom;
        self.pan += new_origin - (self.view.origin + (render::v2([self.doc.width, self.doc.height]) * (self.zoom - new_zoom)) * 0.5);
        self.zoom = new_zoom;
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New…            Ctrl+N").clicked() {
                    self.file_new();
                    ui.close();
                }
                if ui.button("Open…           Ctrl+O").clicked() {
                    self.file_open();
                    ui.close();
                }
                ui.separator();
                if ui.button("Save            Ctrl+S").clicked() {
                    self.file_save(false);
                    ui.close();
                }
                if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                    self.file_save(true);
                    ui.close();
                }
                ui.separator();
                if ui.button("Publish to Flint Hub…  Ctrl+Shift+P").clicked() {
                    self.open_publish();
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Export", |ui| {
                    if ui.button("Export Image (PNG)…").clicked() {
                        self.export("png");
                        ui.close();
                    }
                    if ui.button("Export PNG Sequence…").clicked() {
                        self.export("png_seq");
                        ui.close();
                    }
                    if ui.button("Export Animated GIF…").clicked() {
                        self.export("gif");
                        ui.close();
                    }
                    if ui.button("Export Frame as SVG…").clicked() {
                        self.export("svg");
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Export Flint Movie (.flx)…").clicked() {
                        self.export("flx");
                        ui.close();
                    }
                    if ui.button("Export Projector (standalone app)…").clicked() {
                        self.export("projector");
                        ui.close();
                    }
                });
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo   Ctrl+Z")).clicked() {
                    self.do_undo();
                    ui.close();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo   Ctrl+Y")).clicked() {
                    self.do_redo();
                    ui.close();
                }
                ui.separator();
                let has = !self.sel.is_empty();
                if ui.add_enabled(has, egui::Button::new("Cut     Ctrl+X")).clicked() {
                    self.copy_selection(&ui.ctx().clone());
                    self.delete_selection();
                    ui.close();
                }
                if ui.add_enabled(has, egui::Button::new("Copy    Ctrl+C")).clicked() {
                    self.copy_selection(&ui.ctx().clone());
                    ui.close();
                }
                if ui.button("Paste   Ctrl+V").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    ui.close();
                }
                if ui.add_enabled(has, egui::Button::new("Duplicate  Ctrl+D")).clicked() {
                    self.duplicate();
                    ui.close();
                }
                if ui.add_enabled(has, egui::Button::new("Clear      Del")).clicked() {
                    self.delete_selection();
                    ui.close();
                }
                ui.separator();
                if ui.button("Select All  Ctrl+A").clicked() {
                    self.select_all();
                    ui.close();
                }
                if ui.button("Deselect All  Esc").clicked() {
                    self.sel.clear();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom In     Ctrl+=").clicked() {
                    self.zoom_by(2.0, None);
                }
                if ui.button("Zoom Out    Ctrl+-").clicked() {
                    self.zoom_by(0.5, None);
                }
                if ui.button("100%        Ctrl+1").clicked() {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                if ui.button("Fit in Window  Ctrl+0").clicked() {
                    self.fit_pending = true;
                }
                ui.separator();
                ui.checkbox(&mut self.clip_stage, "Clip to Stage");
                ui.checkbox(&mut self.show_grid, "Show Grid   Ctrl+'");
                ui.checkbox(&mut self.snap_grid, "Snap to Grid  Ctrl+Shift+'");
                ui.horizontal(|ui| {
                    ui.label("Grid size");
                    ui.add(egui::DragValue::new(&mut self.grid_size).range(2.0..=500.0).suffix(" px"));
                });
                ui.checkbox(&mut self.onion, "Onion Skin");
            });
            ui.menu_button("Insert", |ui| {
                if ui.button("New Symbol…   Ctrl+F8").clicked() {
                    self.new_symbol();
                    ui.close();
                }
                if ui.button("New Layer").clicked() {
                    self.add_layer();
                    ui.close();
                }
                ui.separator();
                if ui.button("Frame            F5").clicked() {
                    self.tl_insert_frame();
                    ui.close();
                }
                if ui.button("Keyframe         F6").clicked() {
                    self.tl_keyframe(false);
                    ui.close();
                }
                if ui.button("Blank Keyframe   F7").clicked() {
                    self.tl_keyframe(true);
                    ui.close();
                }
                ui.separator();
                if ui.button("Create Classic Tween").clicked() {
                    self.tl_set_tween(TweenKind::Classic);
                    ui.close();
                }
                if ui.button("Create Shape Tween").clicked() {
                    self.tl_set_tween(TweenKind::Shape);
                    ui.close();
                }
            });
            ui.menu_button("Modify", |ui| {
                let has = !self.sel.is_empty();
                ui.add_enabled_ui(has, |ui| {
                    if ui.button("Convert to Symbol      F8").clicked() {
                        self.convert_to_symbol();
                        ui.close();
                    }
                    if ui.button("Group            Ctrl+G").clicked() {
                        self.group();
                        ui.close();
                    }
                    if ui.button("Ungroup    Ctrl+Shift+G").clicked() {
                        self.ungroup();
                        ui.close();
                    }
                    if ui.button("Distribute to Layers  Ctrl+Shift+D").clicked() {
                        self.distribute_to_layers();
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Arrange", |ui| {
                        for (lbl, v) in [("Bring to Front  Ctrl+Shift+↑", 3), ("Bring Forward  Ctrl+↑", 2), ("Send Backward  Ctrl+↓", 1), ("Send to Back  Ctrl+Shift+↓", 0)] {
                            if ui.button(lbl).clicked() {
                                self.arrange(v);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Transform", |ui| {
                        if ui.button("Flip Horizontal").clicked() {
                            self.flip(true);
                            ui.close();
                        }
                        if ui.button("Flip Vertical").clicked() {
                            self.flip(false);
                            ui.close();
                        }
                        if ui.button("Rotate 90° CW").clicked() {
                            self.rotate_sel(std::f32::consts::FRAC_PI_2);
                            ui.close();
                        }
                        if ui.button("Rotate 90° CCW").clicked() {
                            self.rotate_sel(-std::f32::consts::FRAC_PI_2);
                            ui.close();
                        }
                        if ui.button("Remove Transform").clicked() {
                            self.checkpoint();
                            self.for_selected(|s| {
                                s.xf.sx = 1.0;
                                s.xf.sy = 1.0;
                                s.xf.rot = 0.0;
                            });
                            ui.close();
                        }
                    });
                    ui.menu_button("Align", |ui| {
                        for (lbl, m) in [("Left", 0), ("Horizontal Center", 1), ("Right", 2), ("Top", 3), ("Vertical Center", 4), ("Bottom", 5)] {
                            if ui.button(lbl).clicked() {
                                self.align(m, self.sel.len() == 1);
                                ui.close();
                            }
                        }
                    });
                });
                ui.separator();
                if ui.button("Document…").clicked() {
                    self.new_dialog = None;
                    self.sel.clear();
                    ui.close();
                }
            });
            ui.menu_button("Window", |ui| {
                ui.checkbox(&mut self.show_actions, "Actions   F9");
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Flint & Licenses").clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
            ui.menu_button("Control", |ui| {
                if ui.button(if self.playing { "Stop        Enter" } else { "Play        Enter" }).clicked() {
                    self.playing = !self.playing;
                    ui.close();
                }
                if ui.button("Rewind      Home").clicked() {
                    self.set_frame(0);
                    ui.close();
                }
                if ui.button("Go to End   End").clicked() {
                    self.set_frame(self.doc.frame_count() - 1);
                    ui.close();
                }
                if ui.button("Step Forward   .").clicked() {
                    self.set_frame(self.frame + 1);
                    ui.close();
                }
                if ui.button("Step Backward  ,").clicked() {
                    self.set_frame(self.frame.saturating_sub(1));
                    ui.close();
                }
                ui.separator();
                if ui.button("Test Movie  Ctrl+Enter").clicked() {
                    self.test_movie();
                    ui.close();
                }
                if ui.button("Check Scripts").clicked() {
                    self.check_scripts();
                    ui.close();
                }
                ui.checkbox(&mut self.looping, "Loop Playback");
            });
        });
    }

    fn rotate_sel(&mut self, a: f32) {
        let Some(b) = self.sel_bounds() else { return };
        self.checkpoint();
        let pivot = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        let orig = self.sel_xforms();
        self.set_xforms(&rotated(&orig, pivot, a));
    }

    fn tools_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            for (t, icon, tip, _) in TOOLS {
                let b = egui::Button::selectable(self.tool == *t, egui::RichText::new(*icon).size(18.0)).min_size(Vec2::new(34.0, 30.0));
                if ui.add(b).on_hover_text(*tip).clicked() {
                    self.set_tool(*t);
                }
            }
            ui.separator();
            ui.label("✏").on_hover_text("Stroke color");
            if egui::color_picker::color_edit_button_srgba(ui, &mut self.stroke_col, egui::color_picker::Alpha::OnlyBlend).changed() {
                self.stroke_on = true;
                self.apply_colors_to_sel(true, false);
            }
            ui.label("🪣").on_hover_text("Fill color");
            if egui::color_picker::color_edit_button_srgba(ui, &mut self.fill_col, egui::color_picker::Alpha::OnlyBlend).changed() {
                self.fill_on = true;
                self.apply_colors_to_sel(false, true);
            }
            if ui.small_button("⇄").on_hover_text("Swap colors").clicked() {
                std::mem::swap(&mut self.stroke_col, &mut self.fill_col);
            }
            if ui.small_button("⬛").on_hover_text("Black and white").clicked() {
                self.stroke_col = Color32::BLACK;
                self.fill_col = Color32::WHITE;
            }
            if ui.selectable_label(!self.stroke_on, "∅✏").on_hover_text("No stroke").clicked() {
                self.stroke_on = !self.stroke_on;
            }
            if ui.selectable_label(!self.fill_on, "∅🪣").on_hover_text("No fill").clicked() {
                self.fill_on = !self.fill_on;
            }
        });
    }

    fn apply_colors_to_sel(&mut self, stroke: bool, fill: bool) {
        if self.sel.is_empty() {
            return;
        }
        if !self.prop_active {
            self.checkpoint();
            self.prop_active = true;
        }
        let (sc, fc) = (rgba(self.stroke_col), rgba(self.fill_col));
        self.for_selected(|s| {
            if stroke && !s.is_text() {
                s.stroke = Some(sc);
            }
            if fill && (s.fill.is_some() || s.is_text() || s.local_outline().1) {
                s.fill = Some(fc);
            }
        });
    }

    fn properties_panel(&mut self, ui: &mut Ui) {
        let snapshot = self.doc.clone();
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let sel = self.sel_shapes();
            if sel.is_empty() {
                ui.strong("Document");
                egui::Grid::new("docgrid").num_columns(2).show(ui, |ui| {
                    ui.label("Width");
                    changed |= ui.add(egui::DragValue::new(&mut self.doc.width).range(16.0..=8192.0).suffix(" px")).changed();
                    ui.end_row();
                    ui.label("Height");
                    changed |= ui.add(egui::DragValue::new(&mut self.doc.height).range(16.0..=8192.0).suffix(" px")).changed();
                    ui.end_row();
                    ui.label("FPS");
                    changed |= ui.add(egui::DragValue::new(&mut self.doc.fps).range(1.0..=120.0).speed(0.2)).changed();
                    ui.end_row();
                    ui.label("Stage");
                    let mut c = c32(self.doc.background);
                    if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                        self.doc.background = rgba(c);
                        changed = true;
                    }
                    ui.end_row();
                });
            } else {
                ui.strong(if sel.len() == 1 { shape_kind(&sel[0].1).to_string() } else { format!("{} objects", sel.len()) });
                let (li, s0) = sel[0].clone();
                let frame = self.frame;
                if sel.len() == 1 {
                    ui.horizontal(|ui| {
                        ui.label("Instance name");
                        let mut name = s0.name.clone();
                        if ui.add(egui::TextEdit::singleline(&mut name).hint_text("<none>").desired_width(120.0)).changed() {
                            let id = s0.id;
                            for kf in self.doc.layers[li].keys.values_mut() {
                                for s in kf.shapes.iter_mut().filter(|s| s.id == id) {
                                    s.name = name.clone();
                                }
                            }
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(if s0.on_click.trim().is_empty() { "No click script" } else { "Has click script" });
                        if ui.small_button("Edit in Actions…").clicked() {
                            self.show_actions = true;
                            self.actions_target = ActTarget::Click;
                        }
                    });
                    let s0 = self.doc.layers[li].shapes_at(frame).into_iter().find(|s| s.id == s0.id).unwrap_or(s0);
                    let mut s = s0.clone();
                    let mut c = false;
                    let mut ungroup_now = false;
                    let mut edit_symbol: Option<u64> = None;
                    egui::CollapsingHeader::new("Position and Size").default_open(true).show(ui, |ui| {
                        egui::Grid::new("xf").num_columns(4).show(ui, |ui| {
                            ui.label("X");
                            c |= ui.add(egui::DragValue::new(&mut s.xf.x).speed(1.0)).changed();
                            ui.label("Y");
                            c |= ui.add(egui::DragValue::new(&mut s.xf.y).speed(1.0)).changed();
                            ui.end_row();
                            let mut sx = s.xf.sx * 100.0;
                            let mut sy = s.xf.sy * 100.0;
                            ui.label("Scale X");
                            if ui.add(egui::DragValue::new(&mut sx).suffix("%").speed(0.5)).changed() {
                                s.xf.sx = sx / 100.0;
                                c = true;
                            }
                            ui.label("Y");
                            if ui.add(egui::DragValue::new(&mut sy).suffix("%").speed(0.5)).changed() {
                                s.xf.sy = sy / 100.0;
                                c = true;
                            }
                            ui.end_row();
                            let mut deg = s.xf.rot.to_degrees();
                            ui.label("Rotate");
                            if ui.add(egui::DragValue::new(&mut deg).suffix("°").speed(0.5)).changed() {
                                s.xf.rot = deg.to_radians();
                                c = true;
                            }
                            ui.end_row();
                        });
                    });
                    egui::CollapsingHeader::new("Fill and Stroke").default_open(true).show(ui, |ui| {
                        c |= color_opt(ui, "Stroke", &mut s.stroke);
                        if !s.is_text() {
                            ui.horizontal(|ui| {
                                ui.label("Width");
                                c |= ui.add(egui::Slider::new(&mut s.stroke_w, 0.1..=100.0).logarithmic(true)).changed();
                            });
                        }
                        c |= color_opt(ui, if s.is_text() { "Color" } else { "Fill" }, &mut s.fill);
                    });
                    match &mut s.geom {
                        Geom::Rect { w, h, radius } => {
                            egui::CollapsingHeader::new("Rectangle").default_open(true).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("W");
                                    c |= ui.add(egui::DragValue::new(w).range(0.1..=10000.0)).changed();
                                    ui.label("H");
                                    c |= ui.add(egui::DragValue::new(h).range(0.1..=10000.0)).changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Corner radius");
                                    c |= ui.add(egui::DragValue::new(radius).range(0.0..=5000.0)).changed();
                                });
                            });
                        }
                        Geom::Ellipse { rx, ry } => {
                            egui::CollapsingHeader::new("Oval").default_open(true).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Radius X");
                                    c |= ui.add(egui::DragValue::new(rx).range(0.1..=10000.0)).changed();
                                    ui.label("Y");
                                    c |= ui.add(egui::DragValue::new(ry).range(0.1..=10000.0)).changed();
                                });
                            });
                        }
                        Geom::Text { text, size } => {
                            egui::CollapsingHeader::new("Character").default_open(true).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Size");
                                    c |= ui.add(egui::DragValue::new(size).range(1.0..=1000.0).suffix(" pt")).changed();
                                });
                                c |= ui.add(egui::TextEdit::multiline(text).desired_rows(2)).changed();
                            });
                        }
                        Geom::Path { pts, closed } => {
                            egui::CollapsingHeader::new("Path").default_open(true).show(ui, |ui| {
                                ui.label(format!("{} points", pts.len()));
                                c |= ui.checkbox(closed, "Closed").changed();
                            });
                        }
                        Geom::Group { children } => {
                            ui.label(format!("Group of {} objects", children.len()));
                            if ui.button("Ungroup (Ctrl+Shift+G)").clicked() {
                                ungroup_now = true;
                            }
                        }
                        Geom::Symbol { symbol, first, mode, .. } => {
                            let sym = self.doc.symbol(*symbol).map(|x| (x.name.clone(), x.frame_count()));
                            egui::CollapsingHeader::new("Symbol instance").default_open(true).show(ui, |ui| {
                                let (name, len) = sym.unwrap_or(("(missing symbol)".into(), 1));
                                ui.label(format!("Instance of \"{name}\" ({len} frames)"));
                                ui.horizontal(|ui| {
                                    ui.label("Plays");
                                    egui::ComboBox::from_id_salt("symmode").selected_text(match mode { SymMode::Loop => "Loop", SymMode::Once => "Play once", SymMode::Single => "Single frame" }).show_ui(ui, |ui| {
                                        c |= ui.selectable_value(mode, SymMode::Loop, "Loop").changed();
                                        c |= ui.selectable_value(mode, SymMode::Once, "Play once").changed();
                                        c |= ui.selectable_value(mode, SymMode::Single, "Single frame").changed();
                                    });
                                });
                                ui.horizontal(|ui| {
                                    ui.label("First frame");
                                    let mut f1 = *first + 1;
                                    if ui.add(egui::DragValue::new(&mut f1).range(1..=len)).changed() {
                                        *first = f1 - 1;
                                        c = true;
                                    }
                                });
                                if ui.button("✏ Edit Symbol (double-click)").clicked() {
                                    edit_symbol = Some(*symbol);
                                }
                                ui.label(egui::RichText::new("In the player each instance runs its own timeline; scripts can use .play(), .stop(), .goto_and_play(\"label\").").small().weak());
                            });
                        }
                    }
                    if c {
                        changed = true;
                        if let Some(m) = self.doc.shape_mut(li, frame, s.id) {
                            *m = s;
                        }
                    }
                    if ungroup_now {
                        self.ungroup();
                    }
                    if let Some(id) = edit_symbol {
                        self.enter_symbol(id);
                    }
                } else {
                    let mut stroke = s0.stroke;
                    let mut fill = s0.fill;
                    let mut sw = s0.stroke_w;
                    let cs = color_opt(ui, "Stroke", &mut stroke);
                    let cw = ui.horizontal(|ui| {
                        ui.label("Width");
                        ui.add(egui::Slider::new(&mut sw, 0.1..=100.0).logarithmic(true)).changed()
                    }).inner;
                    let cf = color_opt(ui, "Fill", &mut fill);
                    if cs || cw || cf {
                        changed = true;
                        self.for_selected(|s| {
                            if cs {
                                s.stroke = stroke;
                            }
                            if cw {
                                s.stroke_w = sw;
                            }
                            if cf {
                                s.fill = fill;
                            }
                        });
                    }
                }
                ui.separator();
                ui.strong("Align");
                ui.horizontal_wrapped(|ui| {
                    let to_stage = sel.len() == 1;
                    for (lbl, tip, m) in [("⏴|", "Align left", 0), ("|⏸|", "Align horizontal center", 1), ("|⏵", "Align right", 2), ("⏶", "Align top", 3), ("⏹", "Align vertical center", 4), ("⏷", "Align bottom", 5)] {
                        if ui.button(lbl).on_hover_text(tip).clicked() {
                            self.align(m, to_stage);
                        }
                    }
                });
                ui.label(egui::RichText::new(if sel.len() == 1 { "Aligns to stage" } else { "Aligns to selection" }).small().weak());
            }

            ui.separator();
            ui.strong("Frame");
            let li = self.layer;
            let f = self.frame;
            let l = &self.doc.layers[li];
            if f < l.len {
                let k = l.key_at(f);
                ui.label(format!("Layer \"{}\", keyframe {}", l.name, k + 1));
                let mut label = l.keys[&k].label.clone();
                let mut tw = l.keys[&k].tween;
                ui.horizontal(|ui| {
                    ui.label("Label");
                    if ui.text_edit_singleline(&mut label).changed() {
                        self.doc.layers[li].keys.get_mut(&k).unwrap().label = label.clone();
                        changed = true;
                    }
                });
                let before = tw;
                egui::ComboBox::from_label("Tween").selected_text(format!("{:?}", tw.kind)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut tw.kind, TweenKind::None, "None");
                    ui.selectable_value(&mut tw.kind, TweenKind::Classic, "Classic");
                    ui.selectable_value(&mut tw.kind, TweenKind::Shape, "Shape");
                });
                if tw.kind != TweenKind::None {
                    ui.add(egui::Slider::new(&mut tw.ease, -100.0..=100.0).text("Ease"));
                    if tw.kind == TweenKind::Classic {
                        ui.horizontal(|ui| {
                            ui.label("Rotate");
                            ui.add(egui::DragValue::new(&mut tw.spins).range(-20..=20).suffix(" × (+CW / −CCW)"));
                        });
                    }
                }
                if tw != before {
                    if tw.kind != before.kind {
                        self.tl_set_tween(tw.kind);
                    }
                    self.doc.layers[li].keys.get_mut(&k).unwrap().tween = tw;
                    changed = true;
                }
            } else {
                ui.label("(empty frame)");
            }

            ui.separator();
            ui.strong(format!("Tool: {:?}", self.tool));
            match self.tool {
                Tool::Pencil | Tool::Line | Tool::Rect | Tool::Oval | Tool::Ink => {
                    ui.add(egui::Slider::new(&mut self.stroke_w, 0.1..=100.0).logarithmic(true).text("Stroke"));
                }
                Tool::Brush => {
                    ui.add(egui::Slider::new(&mut self.brush_size, 1.0..=200.0).logarithmic(true).text("Size"));
                }
                Tool::Eraser => {
                    ui.add(egui::Slider::new(&mut self.eraser_size, 2.0..=200.0).logarithmic(true).text("Size"));
                }
                Tool::Text => {
                    ui.add(egui::Slider::new(&mut self.font_size, 4.0..=300.0).logarithmic(true).text("Size"));
                }
                _ => {}
            }
            if matches!(self.tool, Tool::Pencil | Tool::Brush) {
                ui.add(egui::Slider::new(&mut self.smoothing, 0.0..=100.0).text("Smoothing"));
            }
            if self.tool == Tool::Bucket {
                ui.horizontal(|ui| {
                    ui.label("Gap");
                    for (v, l) in [(0.0, "None"), (3.0, "Small"), (7.0, "Medium"), (14.0, "Large")] {
                        ui.selectable_value(&mut self.bucket_gap, v, l);
                    }
                });
                ui.label(egui::RichText::new("Fills the area enclosed by lines; a bigger gap closes small openings between them.").small().weak());
            }
            if self.tool == Tool::Rect {
                ui.add(egui::Slider::new(&mut self.rect_radius, 0.0..=200.0).text("Corner radius"));
            }
        });
        if changed && !self.prop_active {
            self.undo.push(snapshot);
            self.redo.clear();
            self.dirty = true;
            self.prop_active = true;
        }
        let down = ui.input(|i| i.pointer.any_down());
        if !changed && !down {
            self.prop_active = false;
        }
    }

    // ------------------------------------------------------------ timeline

    fn timeline(&mut self, ui: &mut Ui) {
        let total = self.doc.frame_count();
        ui.horizontal(|ui| {
            if ui.button("⏮").on_hover_text("Go to first frame").clicked() {
                self.set_frame(0);
            }
            if ui.button("⏪").on_hover_text("Step back (,)").clicked() {
                self.set_frame(self.frame.saturating_sub(1));
            }
            if ui.button(if self.playing { "⏸" } else { "▶" }).on_hover_text("Play / Stop (Enter)").clicked() {
                self.playing = !self.playing;
            }
            if ui.button("⏩").on_hover_text("Step forward (.)").clicked() {
                self.set_frame(self.frame + 1);
            }
            if ui.button("⏭").on_hover_text("Go to last frame").clicked() {
                self.set_frame(total - 1);
            }
            ui.toggle_value(&mut self.looping, "🔁").on_hover_text("Loop");
            ui.separator();
            ui.toggle_value(&mut self.onion, "🧅 Onion").on_hover_text("Onion skin");
            if self.onion {
                ui.add(egui::DragValue::new(&mut self.onion_range).range(1..=10)).on_hover_text("Onion skin range");
            }
            ui.separator();
            ui.label(format!("Frame {} / {}", self.frame + 1, total));
            ui.label(format!("{:.2}s", self.frame as f32 / self.doc.fps));
            ui.add(egui::DragValue::new(&mut self.doc.fps).range(1.0..=120.0).speed(0.2).suffix(" fps"));
            ui.separator();
            if ui.button("F5 +Frame").clicked() {
                self.tl_insert_frame();
            }
            if ui.button("F6 Keyframe").clicked() {
                self.tl_keyframe(false);
            }
            if ui.button("F7 Blank").clicked() {
                self.tl_keyframe(true);
            }
        });
        ui.separator();

        let row_h = 22.0;
        let ruler_h = 20.0;
        let cell_w = 12.0;
        let col_w = 230.0;
        let n_layers = self.doc.layers.len();

        egui::ScrollArea::vertical().id_salt("tlv").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                // layer column
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    ui.horizontal(|ui| {
                        ui.set_height(ruler_h);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.small_button("➕").on_hover_text("New layer").clicked() {
                            self.add_layer();
                        }
                        if ui.small_button("🗑").on_hover_text("Delete layer").clicked() {
                            self.delete_layer();
                        }
                        if ui.small_button("⏶").on_hover_text("Move layer up").clicked() {
                            self.move_layer(true);
                        }
                        if ui.small_button("⏷").on_hover_text("Move layer down").clicked() {
                            self.move_layer(false);
                        }
                    });
                    for li in 0..self.doc.layers.len() {
                        self.layer_row(ui, li, col_w, row_h);
                    }
                });
                // frame grid
                egui::ScrollArea::horizontal().id_salt("tlh").show(ui, |ui| {
                    let avail = ui.available_width();
                    let shown = ((total + 60) as f32).max(avail / cell_w).ceil() as usize;
                    let size = Vec2::new(shown as f32 * cell_w, ruler_h + n_layers as f32 * row_h);
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                    let p = ui.painter_at(rect);
                    self.draw_grid(&p, rect, shown, cell_w, row_h, ruler_h);
                    let cell_at = |pos: Pos2| {
                        let f = ((pos.x - rect.left()) / cell_w).floor().max(0.0) as usize;
                        let row = ((pos.y - rect.top() - ruler_h) / row_h).floor();
                        (f, if row < 0.0 { None } else { Some((row as usize).min(n_layers - 1)) })
                    };
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let (f, row) = cell_at(pos);
                        if resp.drag_started() || resp.clicked() || resp.secondary_clicked() {
                            if row.is_none() {
                                self.drag = Drag::Scrub;
                            }
                            if let Some(r) = row {
                                if r != self.layer {
                                    self.sel.clear();
                                }
                                self.layer = r;
                            }
                            self.playing = false;
                            self.set_frame(f);
                            self.ctx_cell = (self.layer, f);
                        } else if resp.dragged() {
                            self.set_frame(f);
                        }
                    }
                    if resp.drag_stopped() && matches!(self.drag, Drag::Scrub) {
                        self.drag = Drag::None;
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Create Classic Tween").clicked() {
                            self.tl_set_tween(TweenKind::Classic);
                            ui.close();
                        }
                        if ui.button("Create Shape Tween").clicked() {
                            self.tl_set_tween(TweenKind::Shape);
                            ui.close();
                        }
                        if ui.button("Remove Tween").clicked() {
                            self.tl_set_tween(TweenKind::None);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Insert Frame        F5").clicked() {
                            self.tl_insert_frame();
                            ui.close();
                        }
                        if ui.button("Remove Frames  Shift+F5").clicked() {
                            self.tl_remove_frame();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Insert Keyframe     F6").clicked() {
                            self.tl_keyframe(false);
                            ui.close();
                        }
                        if ui.button("Insert Blank Keyframe F7").clicked() {
                            self.tl_keyframe(true);
                            ui.close();
                        }
                        if ui.button("Clear Keyframe Shift+F6").clicked() {
                            self.tl_clear_keyframe();
                            ui.close();
                        }
                    });
                });
            });
        });
    }

    fn layer_row(&mut self, ui: &mut Ui, li: usize, w: f32, h: f32) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
        let active = li == self.layer;
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, if active { Color32::from_rgb(38, 90, 160) } else { Color32::from_gray(48) });
        p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, Color32::from_gray(30)));
        let l = { let l = &self.doc.layers[li]; LayerInfo { name: l.name.clone(), visible: l.visible, locked: l.locked, outline: l.outline, color: l.color } };
        p.text(rect.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, "📄", FontId::proportional(12.0), Color32::from_gray(200));
        let name_rect = Rect::from_min_max(rect.min + Vec2::new(26.0, 0.0), Pos2::new(rect.right() - 72.0, rect.bottom()));
        if self.renaming == Some(li) {
            let mut name = l.name.clone();
            let r = ui.put(name_rect.shrink(1.0), egui::TextEdit::singleline(&mut name));
            if r.changed() {
                self.doc.layers[li].name = name;
                self.dirty = true;
            }
            r.request_focus();
            if r.lost_focus() {
                self.renaming = None;
            }
        } else {
            p.text(name_rect.left_center(), Align2::LEFT_CENTER, &l.name, FontId::proportional(13.0), Color32::WHITE);
            if active && !self.sel.is_empty() {
                p.text(rect.right_center() - Vec2::new(78.0, 0.0), Align2::RIGHT_CENTER, "✏", FontId::proportional(11.0), Color32::from_gray(220));
            }
        }
        let icon = |x: f32| Rect::from_center_size(Pos2::new(rect.right() - x, rect.center().y), Vec2::splat(18.0));
        let (eye, lock, outl) = (icon(60.0), icon(38.0), icon(16.0));
        let col = |on: bool| if on { Color32::WHITE } else { Color32::from_gray(110) };
        p.text(eye.center(), Align2::CENTER_CENTER, if l.visible { "👁" } else { "✖" }, FontId::proportional(13.0), col(l.visible));
        p.text(lock.center(), Align2::CENTER_CENTER, if l.locked { "🔒" } else { "•" }, FontId::proportional(13.0), col(l.locked));
        let oc = c32(l.color);
        if l.outline {
            p.rect_stroke(outl.shrink(3.0), 0.0, Stroke::new(2.0, oc), StrokeKind::Inside);
        } else {
            p.rect_filled(outl.shrink(3.0), 0.0, oc);
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let l = &mut self.doc.layers[li];
                if eye.contains(pos) {
                    l.visible = !l.visible;
                    self.dirty = true;
                } else if lock.contains(pos) {
                    l.locked = !l.locked;
                    self.dirty = true;
                } else if outl.contains(pos) {
                    l.outline = !l.outline;
                    self.dirty = true;
                } else {
                    if self.layer != li {
                        self.sel.clear();
                    }
                    self.layer = li;
                }
            }
        }
        if resp.double_clicked() {
            self.renaming = Some(li);
        }
    }

    fn draw_grid(&self, p: &egui::Painter, rect: Rect, shown: usize, cw: f32, rh: f32, ruler_h: f32) {
        p.rect_filled(rect, 0.0, Color32::from_gray(34));
        // ruler
        let ruler = Rect::from_min_size(rect.min, Vec2::new(rect.width(), ruler_h));
        p.rect_filled(ruler, 0.0, Color32::from_gray(52));
        for f in 0..shown {
            let x = rect.left() + f as f32 * cw;
            if (f + 1) % 5 == 0 || f == 0 {
                p.line_segment([Pos2::new(x + cw * 0.5, ruler.bottom() - 5.0), Pos2::new(x + cw * 0.5, ruler.bottom())], Stroke::new(1.0, Color32::from_gray(150)));
                p.text(Pos2::new(x + cw * 0.5, ruler.top() + 1.0), Align2::CENTER_TOP, format!("{}", f + 1), FontId::proportional(10.0), Color32::from_gray(190));
            }
        }
        let clip = p.clip_rect();
        let fmin = (((clip.left() - rect.left()) / cw).floor().max(0.0)) as usize;
        let fmax = (((clip.right() - rect.left()) / cw).ceil() as usize + 1).min(shown);
        for (li, l) in self.doc.layers.iter().enumerate() {
            let y = rect.top() + ruler_h + li as f32 * rh;
            let row = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), rh));
            if li == self.layer {
                p.rect_filled(row, 0.0, Color32::from_gray(42));
            }
            for f in fmin..fmax {
                let c = Rect::from_min_size(Pos2::new(rect.left() + f as f32 * cw, y), Vec2::new(cw, rh));
                let in_span = f < l.len;
                let bg = if in_span {
                    match l.tween_at(f).filter(|(k, end, _)| l.next_key(*k).is_some_and(|n| n < l.len && n == *end)) {
                        Some((_, _, tw)) if tw.kind == TweenKind::Classic => Color32::from_rgb(105, 80, 165),
                        Some(_) => Color32::from_rgb(70, 135, 75),
                        None => Color32::from_gray(205),
                    }
                } else if (f + 1) % 5 == 0 {
                    Color32::from_gray(44)
                } else {
                    Color32::from_gray(36)
                };
                p.rect_filled(c, 0.0, bg);
                p.line_segment([c.right_top(), c.right_bottom()], Stroke::new(1.0, if in_span { Color32::from_gray(150) } else { Color32::from_gray(28) }));
            }
            // tween arrows / broken tweens, keyframes, span ends
            for (&k, kf) in &l.keys {
                if k >= l.len {
                    continue;
                }
                let end = l.span_end(k);
                let cx = |f: usize| rect.left() + f as f32 * cw + cw * 0.5;
                let cy = y + rh * 0.5;
                if kf.tween.kind != TweenKind::None {
                    let valid = l.next_key(k).is_some_and(|n| n < l.len);
                    if end > k + 1 {
                        let stroke = Stroke::new(1.0, if valid { Color32::BLACK } else { Color32::from_gray(60) });
                        if valid {
                            p.arrow(Pos2::new(cx(k) + 4.0, cy), Vec2::new(cx(end) - cx(k) - 6.0, 0.0), stroke);
                        } else {
                            p.extend(egui::Shape::dashed_line(&[Pos2::new(cx(k) + 4.0, cy), Pos2::new(cx(end - 1), cy)], stroke, 3.0, 3.0));
                        }
                    }
                }
                let dot = Pos2::new(cx(k), y + rh - 6.0);
                if kf.shapes.is_empty() {
                    p.circle(dot, 3.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
                } else {
                    p.circle_filled(dot, 3.5, Color32::BLACK);
                }
                if !kf.script.trim().is_empty() {
                    p.text(Pos2::new(cx(k), y + 1.0), Align2::CENTER_TOP, "a", FontId::proportional(11.0), Color32::from_rgb(20, 20, 20));
                }
                if !kf.label.is_empty() {
                    p.text(Pos2::new(cx(k) - 3.0, y + 2.0), Align2::LEFT_TOP, format!("⚑{}", kf.label), FontId::proportional(10.0), Color32::from_rgb(180, 20, 20));
                }
                if end > k + 1 && kf.tween.kind == TweenKind::None {
                    let r = Rect::from_center_size(Pos2::new(cx(end - 1), y + rh - 7.0), Vec2::new(5.0, 7.0));
                    p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
                }
            }
            p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, Color32::from_gray(25)));
        }
        // selected cell + playhead
        let sel = Rect::from_min_size(Pos2::new(rect.left() + self.frame as f32 * cw, rect.top() + ruler_h + self.layer as f32 * rh), Vec2::new(cw, rh));
        p.rect_filled(sel, 0.0, Color32::from_rgba_unmultiplied(40, 120, 255, 110));
        let px = rect.left() + self.frame as f32 * cw + cw * 0.5;
        let red = Color32::from_rgb(235, 50, 50);
        p.line_segment([Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())], Stroke::new(1.5, red));
        let tag = Rect::from_center_size(Pos2::new(px, ruler.center().y), Vec2::new(cw.max(22.0), ruler_h - 4.0));
        p.rect_filled(tag, CornerRadius::same(3), red);
        p.text(tag.center(), Align2::CENTER_CENTER, format!("{}", self.frame + 1), FontId::proportional(10.0), Color32::WHITE);
        if self.onion {
            let a = self.frame.saturating_sub(self.onion_range);
            let b = self.frame + self.onion_range;
            let r = Rect::from_min_max(Pos2::new(rect.left() + a as f32 * cw, ruler.bottom() - 4.0), Pos2::new(rect.left() + (b + 1) as f32 * cw, ruler.bottom()));
            p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(120, 170, 255, 140));
        }
    }

    // ------------------------------------------------------------ stage

    fn stage(&mut self, ui: &mut Ui) {
        self.breadcrumb(ui);
        let rect = ui.available_rect_before_wrap();
        self.canvas_center = rect.center();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        if self.fit_pending {
            self.zoom = ((rect.width() - 40.0) / self.doc.width).min((rect.height() - 40.0) / self.doc.height).clamp(0.05, 32.0);
            self.pan = Vec2::ZERO;
            self.fit_pending = false;
        }
        let stage_px = Vec2::new(self.doc.width, self.doc.height) * self.zoom;
        let origin = rect.center() + self.pan - stage_px * 0.5;
        self.view = View { origin, zoom: self.zoom };
        let v = self.view;

        // navigation: wheel / pinch / middle-drag / space-drag
        if resp.hovered() {
            let (scroll, zd, hover) = ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            if zd != 1.0 {
                self.zoom_by(zd, hover);
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        let space = !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_down(Key::Space));
        if resp.dragged_by(egui::PointerButton::Middle) || ((space || matches!(self.drag, Drag::Pan)) && resp.dragged_by(egui::PointerButton::Primary)) {
            self.pan += resp.drag_delta();
        }

        // ---- draw
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(70));
        let stage_rect = Rect::from_min_size(origin, stage_px);
        let in_symbol = !self.doc.edit_stack.is_empty();
        if in_symbol {
            // symbol space: no stage, just the registration point at (0,0)
            painter.rect_filled(rect, 0.0, c32(self.doc.background).gamma_multiply(0.92));
        } else {
            painter.rect_filled(stage_rect.translate(Vec2::splat(3.0)), 0.0, Color32::from_black_alpha(80));
            painter.rect_filled(stage_rect, 0.0, c32(self.doc.background));
        }
        if self.show_grid {
            let g = self.grid_size.max(2.0) * self.zoom;
            if g >= 4.0 {
                let gp = painter.with_clip_rect(stage_rect.intersect(rect));
                let col = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 140, 170, 70));
                let mut x = stage_rect.left();
                while x <= stage_rect.right() {
                    gp.line_segment([Pos2::new(x, stage_rect.top()), Pos2::new(x, stage_rect.bottom())], col);
                    x += g;
                }
                let mut y = stage_rect.top();
                while y <= stage_rect.bottom() {
                    gp.line_segment([Pos2::new(stage_rect.left(), y), Pos2::new(stage_rect.right(), y)], col);
                    y += g;
                }
            }
        }
        let shape_painter = if self.clip_stage { painter.with_clip_rect(stage_rect.intersect(rect)) } else { painter.clone() };

        if self.onion && !self.playing {
            for d in (1..=self.onion_range).rev() {
                let a = 0.35 * (1.0 - (d - 1) as f32 / self.onion_range as f32);
                for f in [self.frame.checked_sub(d), Some(self.frame + d)].into_iter().flatten() {
                    for (_, s) in self.doc.render_list(f) {
                        render::draw_shape(&shape_painter, &v, &s, &render::Style { alpha: a, outline: None });
                    }
                }
            }
        }
        for (li, s) in self.doc.render_list(self.frame) {
            if self.text_edit == Some((li, s.id)) {
                continue;
            }
            let l = &self.doc.layers[li];
            let outline = l.outline.then(|| c32(l.color));
            render::draw_shape(&shape_painter, &v, &s, &render::Style { alpha: 1.0, outline });
        }

        if in_symbol {
            let o = v.to_screen([0.0, 0.0]);
            let k = Stroke::new(1.0, Color32::from_gray(30));
            painter.line_segment([o - Vec2::new(8.0, 0.0), o + Vec2::new(8.0, 0.0)], k);
            painter.line_segment([o - Vec2::new(0.0, 8.0), o + Vec2::new(0.0, 8.0)], k);
            painter.circle_stroke(o, 3.0, k);
        }
        // selection decorations
        let accent = Color32::from_rgb(40, 140, 255);
        if !self.playing {
            for (li, s) in self.sel_shapes() {
                let b = s.bounds();
                let r = Rect::from_min_max(v.to_screen([b[0], b[1]]), v.to_screen([b[2], b[3]])).expand(2.0);
                let col = c32(self.doc.layers[li].color);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, col), StrokeKind::Outside);
            }
            if self.tool == Tool::Transform {
                if let Some(b) = self.sel_bounds() {
                    let r = Rect::from_min_max(v.to_screen([b[0], b[1]]), v.to_screen([b[2], b[3]]));
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Middle);
                    for h in handles(r) {
                        let hr = Rect::from_center_size(h, Vec2::splat(7.0));
                        painter.rect_filled(hr, 0.0, Color32::WHITE);
                        painter.rect_stroke(hr, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
                    }
                    painter.circle(r.center(), 4.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
                }
            }
        }

        // ---- tool interaction
        let hover = resp.hover_pos();
        let (pressed, down, released, mods) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.modifiers));
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        if space || matches!(self.drag, Drag::Pan) || self.tool == Tool::Hand {
            ctx.set_cursor_icon(if down { CursorIcon::Grabbing } else { CursorIcon::Grab });
        } else if resp.hovered() {
            ctx.set_cursor_icon(match self.tool {
                Tool::Select => CursorIcon::Default,
                Tool::Transform => self.transform_cursor(hover),
                Tool::Text => CursorIcon::Text,
                Tool::Zoom => CursorIcon::ZoomIn,
                _ => CursorIcon::Crosshair,
            });
        }

        if resp.double_clicked() && self.tool == Tool::Select && !space {
            if let Some(pos) = hover {
                match self.hit_test(v.to_stage(pos), None).and_then(|(_, s)| s.symbol_id()) {
                    Some(id) => {
                        self.drag = Drag::None;
                        self.enter_symbol(id);
                        return;
                    }
                    None if in_symbol && self.hit_test(v.to_stage(pos), None).is_none() => {
                        self.exit_symbol();
                        return;
                    }
                    None => {}
                }
            }
        }
        if pressed && resp.contains_pointer() && !space {
            if let Some(pos) = hover {
                self.on_press(pos, mods, &ctx);
            }
        }
        if down && !space {
            if let Some(pos) = pointer {
                self.on_drag(pos, mods);
            }
        }
        if released {
            if let Some(pos) = pointer {
                self.on_release(pos, mods);
            }
            if !matches!(self.drag, Drag::Scrub) {
                self.drag = Drag::None;
            }
        }

        // drag previews
        match &self.drag {
            Drag::Draw(pts) if pts.len() > 1 => {
                let sp: Vec<Pos2> = pts.iter().map(|p| v.to_screen(*p)).collect();
                let (col, w) = if self.tool == Tool::Brush { (self.fill_col, self.brush_size) } else { (self.stroke_col, self.stroke_w) };
                render::stroke_polyline(&painter, &sp, false, (w * self.zoom).max(1.0), col);
            }
            Drag::Box(a, b) => {
                if let Some(s) = self.box_shape(*a, *b, mods) {
                    render::draw_shape(&painter, &v, &s, &render::Style { alpha: 1.0, outline: None });
                }
            }
            Drag::Marquee(a, b) => {
                let r = Rect::from_two_pos(v.to_screen(*a), v.to_screen(*b));
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(40, 140, 255, 30));
                render::dashed_rect(&painter, r, accent);
            }
            _ => {}
        }
        if self.tool == Tool::Eraser && !space {
            if let Some(h) = hover {
                painter.circle_stroke(h, self.eraser_size * 0.5 * self.zoom, Stroke::new(1.0, Color32::from_gray(20)));
            }
        }

        self.text_overlay(ui);

        // status overlay: stage coords
        if let Some(h) = hover {
            let sp = v.to_stage(h);
            painter.text(rect.left_bottom() + Vec2::new(8.0, -6.0), Align2::LEFT_BOTTOM, format!("x {:.0}  y {:.0}   {:.0}%", sp[0], sp[1], self.zoom * 100.0), FontId::monospace(11.0), Color32::from_gray(220));
        }
    }

    fn transform_cursor(&self, hover: Option<Pos2>) -> CursorIcon {
        let (Some(h), Some(b)) = (hover, self.sel_bounds()) else { return CursorIcon::Default };
        let r = Rect::from_min_max(self.view.to_screen([b[0], b[1]]), self.view.to_screen([b[2], b[3]]));
        match transform_hit(r, h) {
            Some(TH::Scale(i)) => [CursorIcon::ResizeNwSe, CursorIcon::ResizeVertical, CursorIcon::ResizeNeSw, CursorIcon::ResizeHorizontal, CursorIcon::ResizeNwSe, CursorIcon::ResizeVertical, CursorIcon::ResizeNeSw, CursorIcon::ResizeHorizontal][i],
            Some(TH::Rotate) => CursorIcon::Alias,
            None => CursorIcon::Default,
        }
    }

    fn on_press(&mut self, pos: Pos2, mods: Modifiers, ctx: &egui::Context) {
        let v = self.view;
        let sp = v.to_stage(pos);
        if self.text_edit.is_some() {
            self.finish_text();
            if self.tool == Tool::Text {
                return;
            }
        }
        if ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle)) {
            return;
        }
        match self.tool {
            Tool::Hand => self.drag = Drag::Pan,
            Tool::Zoom => self.zoom_by(if mods.alt { 0.5 } else { 2.0 }, Some(pos)),
            Tool::Select | Tool::Transform => {
                if self.tool == Tool::Transform {
                    if let Some(b) = self.sel_bounds() {
                        let r = Rect::from_min_max(v.to_screen([b[0], b[1]]), v.to_screen([b[2], b[3]]));
                        match transform_hit(r, pos) {
                            Some(TH::Scale(i)) => {
                                self.checkpoint();
                                let hs = handles(r);
                                let pivot = if mods.alt { v.to_stage(r.center()) } else { v.to_stage(hs[(i + 4) % 8]) };
                                self.drag = Drag::Scale { handle: i, pivot, start: sp, orig: self.sel_xforms() };
                                return;
                            }
                            Some(TH::Rotate) => {
                                self.checkpoint();
                                self.drag = Drag::Rotate { pivot: v.to_stage(r.center()), start: sp, orig: self.sel_xforms() };
                                return;
                            }
                            None => {}
                        }
                    }
                }
                match self.hit_test(sp, None) {
                    Some((li, s)) => {
                        let key = (li, s.id);
                        if mods.shift {
                            if let Some(i) = self.sel.iter().position(|k| *k == key) {
                                self.sel.remove(i);
                                return;
                            }
                            self.sel.push(key);
                        } else if !self.sel.contains(&key) {
                            self.sel = vec![key];
                        }
                        self.layer = li;
                        if mods.alt {
                            self.duplicate();
                        } else {
                            self.checkpoint();
                        }
                        let b = self.sel_bounds().unwrap_or([sp[0], sp[1], sp[0], sp[1]]);
                        self.drag = Drag::Move { start: sp, orig: self.sel_xforms(), corner: [b[0], b[1]] };
                    }
                    None => {
                        if !mods.shift {
                            self.sel.clear();
                        }
                        self.drag = Drag::Marquee(sp, sp);
                    }
                }
            }
            Tool::Pencil | Tool::Brush => {
                if self.layer_editable(self.layer) {
                    self.drag = Drag::Draw(vec![sp]);
                } else {
                    self.status = "Active layer is locked or hidden".into();
                }
            }
            Tool::Line | Tool::Rect | Tool::Oval => {
                let sp = self.snap(sp);
                self.drag = Drag::Box(sp, sp);
            }
            Tool::Bucket => self.bucket_click(sp),
            Tool::Ink => {
                let target = self.hit_test(sp, None);
                if let Some((li, s)) = target {
                    self.checkpoint();
                    let (sc, sw) = (rgba(self.stroke_col), self.stroke_w);
                    // inside groups, only paint the leaf under the cursor
                    let tol = 2.0 / self.zoom;
                    let target_leaf = s.flatten().iter().rposition(|l| l.hit(sp, tol)).unwrap_or(0);
                    if let Some(m) = self.doc.shape_mut(li, self.frame, s.id) {
                        let mut i = 0;
                        m.for_leaves_mut(&mut |m| {
                            if i == target_leaf && !m.is_text() {
                                m.stroke = Some(sc);
                                m.stroke_w = sw;
                            }
                            i += 1;
                        });
                    }
                }
            }
            Tool::Dropper => {
                if let Some((_, s)) = self.hit_test(sp, None) {
                    let (pts, closed) = s.world_outline();
                    let near_stroke = polyline_dist(sp, &pts, closed) <= s.stroke_w * s.xf.avg_scale() * 0.5 + 4.0 / self.zoom;
                    if s.stroke.is_some() && (near_stroke || s.fill.is_none()) {
                        self.stroke_col = c32(s.stroke.unwrap());
                        self.stroke_w = s.stroke_w;
                        self.stroke_on = true;
                        self.tool = Tool::Ink;
                    } else if let Some(f) = s.fill {
                        self.fill_col = c32(f);
                        self.fill_on = true;
                        self.tool = Tool::Bucket;
                    }
                }
            }
            Tool::Eraser => {
                if self.layer_editable(self.layer) {
                    self.checkpoint();
                    self.drag = Drag::Erase;
                    self.erase_at(sp);
                }
            }
            Tool::Text => {
                if let Some((li, s)) = self.hit_test(sp, Some(self.layer)).filter(|(_, s)| s.is_text()) {
                    self.checkpoint();
                    self.text_edit = Some((li, s.id));
                    self.sel = vec![(li, s.id)];
                } else if self.layer_editable(self.layer) {
                    let s = Shape {
                        id: 0,
                        geom: Geom::Text { text: String::new(), size: self.font_size },
                        xf: Xform::at(sp[0], sp[1] - self.font_size * 0.6),
                        stroke: None,
                        stroke_w: 1.0,
                        fill: Some(rgba(self.fill_col)),
                        ..Default::default()
                    };
                    self.add_shape(s);
                    self.text_edit = self.sel.first().copied();
                }
                self.text_focus = true;
            }
        }
    }

    fn on_drag(&mut self, pos: Pos2, mods: Modifiers) {
        let v = self.view;
        let sp = v.to_stage(pos);
        match &mut self.drag {
            Drag::Pan => {}
            Drag::Draw(pts) => {
                let last = *pts.last().unwrap();
                if ((sp[0] - last[0]).powi(2) + (sp[1] - last[1]).powi(2)).sqrt() * self.zoom > 1.5 {
                    pts.push(sp);
                }
            }
            Drag::Box(_, b) => *b = if self.snap_grid { snap_to(sp, self.grid_size) } else { sp },
            Drag::Marquee(_, b) => *b = sp,
            Drag::Move { start, orig, corner } => {
                let (mut dx, mut dy) = (sp[0] - start[0], sp[1] - start[1]);
                if mods.shift {
                    if dx.abs() > dy.abs() { dy = 0.0 } else { dx = 0.0 }
                }
                if self.snap_grid {
                    // snap the selection's top-left corner to the grid
                    let g = self.grid_size.max(1.0);
                    dx = ((corner[0] + dx) / g).round() * g - corner[0];
                    dy = ((corner[1] + dy) / g).round() * g - corner[1];
                }
                let xs: Vec<_> = orig.iter().map(|(li, id, xf)| (*li, *id, Xform { x: xf.x + dx, y: xf.y + dy, ..*xf })).collect();
                self.set_xforms(&xs);
            }
            Drag::Scale { handle, pivot, start, orig } => {
                let (h, pv, st) = (*handle, *pivot, *start);
                let fx_on = !matches!(h, 1 | 5);
                let fy_on = !matches!(h, 3 | 7);
                let ratio = |a: f32, b: f32| if b.abs() < 1e-3 { 1.0 } else { a / b };
                let mut fx = if fx_on { ratio(sp[0] - pv[0], st[0] - pv[0]) } else { 1.0 };
                let mut fy = if fy_on { ratio(sp[1] - pv[1], st[1] - pv[1]) } else { 1.0 };
                if mods.shift && fx_on && fy_on {
                    let u = if fx.abs() > fy.abs() { fx } else { fy };
                    fx = u.abs() * fx.signum();
                    fy = u.abs() * fy.signum();
                }
                let xs: Vec<_> = orig
                    .iter()
                    .map(|(li, id, xf)| {
                        let (s, c) = xf.rot.sin_cos();
                        let lsx = ((fx * c).powi(2) + (fy * s).powi(2)).sqrt() * fx.signum();
                        let lsy = ((fx * s).powi(2) + (fy * c).powi(2)).sqrt() * fy.signum();
                        (*li, *id, Xform { x: pv[0] + (xf.x - pv[0]) * fx, y: pv[1] + (xf.y - pv[1]) * fy, sx: xf.sx * lsx, sy: xf.sy * lsy, rot: xf.rot })
                    })
                    .collect();
                self.set_xforms(&xs);
            }
            Drag::Rotate { pivot, start, orig } => {
                let a0 = (start[1] - pivot[1]).atan2(start[0] - pivot[0]);
                let a1 = (sp[1] - pivot[1]).atan2(sp[0] - pivot[0]);
                let mut d = a1 - a0;
                if mods.shift {
                    let step = 15f32.to_radians();
                    d = (d / step).round() * step;
                }
                let xs = rotated(orig, *pivot, d);
                self.set_xforms(&xs);
            }
            Drag::Erase => self.erase_at(sp),
            Drag::None | Drag::Scrub => {}
        }
    }

    fn on_release(&mut self, pos: Pos2, mods: Modifiers) {
        let sp = self.view.to_stage(pos);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Draw(mut pts) => {
                if pts.len() == 1 {
                    pts.push([pts[0][0] + 0.1, pts[0][1]]);
                }
                let brush = self.tool == Tool::Brush;
                let eps = (0.3 + self.smoothing / 100.0 * 2.2) / self.zoom;
                let mut simp = simplify(&pts, eps);
                let first = pts[0];
                let last = *pts.last().unwrap();
                let close_dist = ((first[0] - last[0]).powi(2) + (first[1] - last[1]).powi(2)).sqrt() * self.zoom;
                let closed = !brush && pts.len() > 6 && close_dist < 12.0;
                if closed && simp.len() > 2 {
                    simp.pop();
                }
                if self.smoothing > 0.0 && simp.len() > 2 {
                    simp = chaikin(&simp, closed, if self.smoothing > 60.0 { 3 } else { 2 });
                }
                let b = bounds_of(&simp);
                let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
                let local = simp.iter().map(|p| [p[0] - c[0], p[1] - c[1]]).collect();
                let s = Shape {
                    id: 0,
                    geom: Geom::Path { pts: local, closed },
                    xf: Xform::at(c[0], c[1]),
                    stroke: Some(if brush { rgba(self.fill_col) } else { rgba(self.stroke_col) }),
                    stroke_w: if brush { self.brush_size } else { self.stroke_w },
                    fill: None,
                    ..Default::default()
                };
                self.add_shape(s);
                self.sel.clear();
            }
            Drag::Box(a, b) => {
                if let Some(s) = self.box_shape(a, b, mods) {
                    self.add_shape(s);
                    self.sel.clear();
                }
            }
            Drag::Marquee(a, b) => {
                let r = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
                if r[2] - r[0] < 1.0 / self.zoom && r[3] - r[1] < 1.0 / self.zoom {
                    return;
                }
                for (li, l) in self.doc.layers.iter().enumerate() {
                    if !l.visible || l.locked {
                        continue;
                    }
                    for s in self.doc.layer_shapes(li, self.frame) {
                        let sb = s.bounds();
                        if sb[0] >= r[0] && sb[1] >= r[1] && sb[2] <= r[2] && sb[3] <= r[3] && !self.sel.contains(&(li, s.id)) {
                            self.sel.push((li, s.id));
                        }
                    }
                }
            }
            Drag::Move { start, .. } => {
                if start == sp && self.undo.last().is_some() {
                    // pure click: drop the no-op checkpoint
                    self.undo.pop();
                }
            }
            _ => {}
        }
    }

    fn box_shape(&self, a: P, b: P, mods: Modifiers) -> Option<Shape> {
        let (mut dx, mut dy) = (b[0] - a[0], b[1] - a[1]);
        if dx.abs() * self.zoom < 2.0 && dy.abs() * self.zoom < 2.0 {
            return None;
        }
        if self.tool == Tool::Line {
            if mods.shift {
                let ang = dy.atan2(dx);
                let step = std::f32::consts::FRAC_PI_4;
                let snapped = (ang / step).round() * step;
                let len = (dx * dx + dy * dy).sqrt();
                dx = snapped.cos() * len;
                dy = snapped.sin() * len;
            }
            let c = [a[0] + dx * 0.5, a[1] + dy * 0.5];
            return Some(Shape {
                id: 0,
                geom: Geom::Path { pts: vec![[-dx * 0.5, -dy * 0.5], [dx * 0.5, dy * 0.5]], closed: false },
                xf: Xform::at(c[0], c[1]),
                stroke: Some(rgba(self.stroke_col)),
                stroke_w: self.stroke_w,
                fill: None,
                ..Default::default()
            });
        }
        if mods.shift {
            let m = dx.abs().max(dy.abs());
            dx = m * dx.signum();
            dy = m * dy.signum();
        }
        let (c, w, h) = if mods.alt { (a, dx.abs() * 2.0, dy.abs() * 2.0) } else { ([a[0] + dx * 0.5, a[1] + dy * 0.5], dx.abs(), dy.abs()) };
        let geom = if self.tool == Tool::Rect { Geom::Rect { w, h, radius: self.rect_radius } } else { Geom::Ellipse { rx: w * 0.5, ry: h * 0.5 } };
        let stroke = self.cur_stroke();
        let fill = self.cur_fill();
        if stroke.is_none() && fill.is_none() {
            return None;
        }
        Some(Shape { geom, xf: Xform::at(c[0], c[1]), stroke, stroke_w: self.stroke_w, fill, ..Default::default() })
    }

    fn erase_at(&mut self, sp: P) {
        let r = self.eraser_size * 0.5;
        let li = self.layer;
        let frame = self.frame;
        // check quickly whether anything is touched before forcing a keyframe edit
        let touched = self.doc.layer_shapes(li, frame).iter().any(|s| s.hit(sp, r));
        if !touched {
            return;
        }
        let mut next_id = self.doc.next_id;
        let key = self.doc.edit_key(li, frame);
        let mut out = Vec::new();
        for s in key.shapes.drain(..) {
            if !s.hit(sp, r) {
                out.push(s);
                continue;
            }
            match &s.geom {
                Geom::Path { pts, closed } if s.fill.is_none() => {
                    // densify, then split the stroke where it passes under the eraser
                    let mut world: Vec<P> = pts.iter().map(|p| s.xf.apply(*p)).collect();
                    if *closed && !world.is_empty() {
                        world.push(world[0]);
                    }
                    let total: f32 = world.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum();
                    let n = ((total / (r * 0.25).max(0.5)) as usize).clamp(2, 4000);
                    let dense = resample(&world, false, n);
                    let mut runs: Vec<Vec<P>> = vec![Vec::new()];
                    for p in dense {
                        let d = ((p[0] - sp[0]).powi(2) + (p[1] - sp[1]).powi(2)).sqrt();
                        if d <= r + s.stroke_w * s.xf.avg_scale() * 0.5 {
                            if !runs.last().unwrap().is_empty() {
                                runs.push(Vec::new());
                            }
                        } else {
                            runs.last_mut().unwrap().push(p);
                        }
                    }
                    for run in runs.into_iter().filter(|r| r.len() >= 2) {
                        let run = simplify(&run, 0.3);
                        let inv: Vec<P> = run.iter().map(|p| s.xf.inverse(*p)).collect();
                        next_id += 1;
                        out.push(Shape { id: next_id, geom: Geom::Path { pts: inv, closed: false }, ..s.clone() });
                    }
                }
                _ => {} // filled shapes / primitives / text are erased whole
            }
        }
        key.shapes = out;
        self.doc.next_id = next_id;
        self.prune_sel();
    }

    fn text_overlay(&mut self, ui: &mut Ui) {
        let Some((li, id)) = self.text_edit else { return };
        let frame = self.frame;
        let Some(s) = self.doc.layers[li].shapes_at(frame).into_iter().find(|s| s.id == id) else {
            self.text_edit = None;
            return;
        };
        let Geom::Text { text, size } = &s.geom else { return };
        let mut buf = text.clone();
        let pos = self.view.to_screen([s.xf.x, s.xf.y]);
        let px = (size * s.xf.sy.abs() * self.zoom).max(4.0);
        let col = c32(s.fill.unwrap_or([0, 0, 0, 255]));
        let mut done = false;
        let width = (text_extent(&buf, *size).0 * self.zoom + px).max(px * 2.0);
        egui::Area::new(egui::Id::unique("text_edit_overlay")).fixed_pos(pos - Vec2::new(3.0, 3.0)).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            egui::Frame::new().stroke(Stroke::new(1.0, Color32::from_rgb(40, 140, 255))).fill(Color32::from_white_alpha(30)).inner_margin(2.0).show(ui, |ui| {
                let te = egui::TextEdit::multiline(&mut buf)
                    .font(FontId::proportional(px))
                    .text_color(col)
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(width);
                let r = ui.add(te);
                if self.text_focus {
                    r.request_focus();
                    self.text_focus = false;
                }
                if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    done = true;
                }
            });
        });
        if buf != *text {
            if let Some(m) = self.doc.shape_mut(li, frame, id) {
                m.geom = Geom::Text { text: buf, size: *size };
            }
            self.dirty = true;
        }
        if done {
            self.finish_text();
        }
    }

    fn new_doc_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut w, mut h, mut fps)) = self.new_dialog else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New Document").collapsible(false).resizable(false).open(&mut open).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (name, pw, ph) in [("HD 1280×720", 1280.0, 720.0), ("Full HD 1920×1080", 1920.0, 1080.0), ("Square 1080", 1080.0, 1080.0), ("Story 1080×1920", 1080.0, 1920.0), ("Classic 550×400", 550.0, 400.0)] {
                    if ui.button(name).clicked() {
                        w = pw;
                        h = ph;
                    }
                }
            });
            egui::Grid::new("newdoc").show(ui, |ui| {
                ui.label("Width");
                ui.add(egui::DragValue::new(&mut w).range(16.0..=8192.0));
                ui.end_row();
                ui.label("Height");
                ui.add(egui::DragValue::new(&mut h).range(16.0..=8192.0));
                ui.end_row();
                ui.label("Frame rate");
                ui.add(egui::DragValue::new(&mut fps).range(1.0..=120.0));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                for f in [12.0, 24.0, 30.0, 60.0] {
                    if ui.small_button(format!("{f} fps")).clicked() {
                        fps = f;
                    }
                }
            });
            ui.separator();
            if self.dirty {
                ui.colored_label(Color32::from_rgb(255, 180, 80), "Unsaved changes in the current document will be lost.");
            }
            if ui.button("Create").clicked() {
                create = true;
            }
        });
        self.new_dialog = if open && !create { Some((w, h, fps)) } else { None };
        if create {
            let (tool, sc, fc) = (self.tool, self.stroke_col, self.fill_col);
            *self = Self::blank(Document::new(w, h, fps));
            self.tool = tool;
            self.stroke_col = sc;
            self.fill_col = fc;
        }
    }
}

struct LayerInfo {
    name: String,
    visible: bool,
    locked: bool,
    outline: bool,
    color: Rgba,
}

enum TH {
    Scale(usize),
    Rotate,
}

/// 8 handles clockwise from top-left: TL, T, TR, R, BR, B, BL, L
fn handles(r: Rect) -> [Pos2; 8] {
    [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()]
}

fn transform_hit(r: Rect, p: Pos2) -> Option<TH> {
    let hs = handles(r);
    for (i, h) in hs.iter().enumerate() {
        if (p - *h).length() <= 6.0 {
            return Some(TH::Scale(i));
        }
    }
    for i in [0, 2, 4, 6] {
        let d = (p - hs[i]).length();
        if d > 6.0 && d < 22.0 && !r.contains(p) {
            return Some(TH::Rotate);
        }
    }
    None
}

fn rotated(orig: &[(usize, u64, Xform)], pivot: P, d: f32) -> Vec<(usize, u64, Xform)> {
    let (s, c) = d.sin_cos();
    orig.iter()
        .map(|(li, id, xf)| {
            let (ox, oy) = (xf.x - pivot[0], xf.y - pivot[1]);
            (*li, *id, Xform { x: pivot[0] + ox * c - oy * s, y: pivot[1] + ox * s + oy * c, rot: xf.rot + d, ..*xf })
        })
        .collect()
}

fn shape_kind(s: &Shape) -> &'static str {
    match s.geom {
        Geom::Path { closed: true, .. } => "Shape",
        Geom::Path { pts: ref p, .. } if p.len() == 2 => "Line",
        Geom::Path { .. } => "Stroke",
        Geom::Rect { .. } => "Rectangle",
        Geom::Ellipse { .. } => "Oval",
        Geom::Text { .. } => "Text",
        Geom::Group { .. } => "Group",
        Geom::Symbol { .. } => "Symbol instance",
    }
}

fn color_opt(ui: &mut Ui, label: &str, c: &mut Option<Rgba>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let mut on = c.is_some();
        if ui.checkbox(&mut on, label).changed() {
            *c = if on { Some([0, 0, 0, 255]) } else { None };
            changed = true;
        }
        if let Some(col) = c {
            let mut cc = c32(*col);
            if egui::color_picker::color_edit_button_srgba(ui, &mut cc, egui::color_picker::Alpha::OnlyBlend).changed() {
                *col = rgba(cc);
                changed = true;
            }
        }
    });
    changed
}

impl eframe::App for FlintApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if let Some(intro) = &mut self.intro {
            let mut done = false;
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| done = intro.show(ui));
            if !done {
                return;
            }
            self.intro = None;
        }
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);

        if self.playing {
            let dt = ctx.input(|i| i.stable_dt).min(0.25) as f64;
            self.play_accum += dt;
            let step = 1.0 / self.doc.fps.max(1.0) as f64;
            let total = self.doc.frame_count();
            while self.play_accum >= step {
                self.play_accum -= step;
                if self.frame + 1 >= total {
                    if self.looping {
                        self.frame = 0;
                    } else {
                        self.playing = false;
                        break;
                    }
                } else {
                    self.frame += 1;
                }
            }
            self.prune_sel();
            ctx.request_repaint();
        } else {
            self.play_accum = 0.0;
        }

        let title = if self.path.is_none() && !self.dirty {
            "Flint — One app, lots".to_string()
        } else {
            format!("{}{} - Flint", self.base_name(), if self.dirty { "*" } else { "" })
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").exact_size(20.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(&self.status).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{:.0}×{:.0} @ {} fps", self.doc.width, self.doc.height, self.doc.fps)).small().weak());
                });
            });
        });
        egui::Panel::bottom("timeline").resizable(true).default_size(230.0).min_size(120.0).show(ui, |ui| self.timeline(ui));
        egui::Panel::left("tools").exact_size(52.0).resizable(false).show(ui, |ui| self.tools_panel(ui));
        egui::Panel::right("props").resizable(true).default_size(280.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.right_tab, 0, "Properties");
                ui.selectable_value(&mut self.right_tab, 1, format!("Library ({})", self.doc.symbols.len()));
            });
            ui.separator();
            if self.right_tab == 1 {
                self.library_panel(ui);
            } else {
                self.properties_panel(ui);
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.stage(ui));
        self.new_doc_dialog(&ctx);
        self.actions_window(&ctx);
        self.publish_window(&ctx);
        crate::about::window(&ctx, &mut self.show_about, "Flint");
    }
}


fn snap_to(p: P, g: f32) -> P {
    let g = g.max(1.0);
    [(p[0] / g).round() * g, (p[1] / g).round() * g]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActTarget {
    Frame,
    Click,
    Start,
    EveryFrame,
    Key,
    Functions,
}

impl FlintApp {
    /// Publishes to a temp .flx and opens it in flint-player (scripts run there).
    fn test_movie(&mut self) {
        self.finish_text();
        self.check_scripts();
        let Some(player) = flx::player_exe() else {
            self.set_frame(0);
            self.playing = true;
            self.looping = true;
            self.status = "flint-player not found next to flint; previewing the timeline without scripts".into();
            return;
        };
        let path = std::env::temp_dir().join(format!("flint-test-{}.flx", std::process::id()));
        match flx::save(&self.doc.normalized(), &path).and_then(|_| std::process::Command::new(&player).arg(&path).spawn().map_err(|e| e.to_string())) {
            Ok(_) => {
                if self.script_errors.is_empty() {
                    self.status = "Testing movie in Flint Player".into();
                }
            }
            Err(e) => self.status = format!("Test Movie failed: {e}"),
        }
    }

    fn check_scripts(&mut self) {
        self.script_errors = runtime::check_scripts(&self.doc.normalized());
        self.status = match self.script_errors.len() {
            0 => "Scripts OK".into(),
            n => {
                self.show_actions = true;
                format!("{n} script error(s) — see Actions panel")
            }
        };
    }

    fn actions_window(&mut self, ctx: &egui::Context) {
        if !self.show_actions {
            self.script_active = false;
            return;
        }
        let mut open = true;
        let snapshot_needed = !self.script_active;
        let snapshot = if snapshot_needed { Some(self.doc.clone()) } else { None };
        let mut changed = false;
        let mut focused = false;
        egui::Window::new("Actions").open(&mut open).default_size([560.0, 420.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (t, lbl) in [(ActTarget::Frame, "Frame"), (ActTarget::Click, "Object: on click"), (ActTarget::Start, "Movie: on start"), (ActTarget::EveryFrame, "Movie: every frame"), (ActTarget::Key, "Movie: on key"), (ActTarget::Functions, "Movie: functions")] {
                    ui.selectable_value(&mut self.actions_target, t, lbl);
                }
            });
            ui.separator();
            let li = self.layer;
            let frame = self.frame;
            let (title, src): (String, Option<String>) = match self.actions_target {
                ActTarget::Frame => {
                    let l = &self.doc.layers[li];
                    if frame < l.len {
                        let k = l.key_at(frame);
                        (format!("Layer \"{}\", keyframe {} — runs when the playhead enters it", l.name, k + 1), Some(l.keys[&k].script.clone()))
                    } else {
                        ("Select a frame inside the layer's span".into(), None)
                    }
                }
                ActTarget::Click => match self.sel_shapes().as_slice() {
                    [(_, s)] => (
                        format!("On click of {} — `me` is this object", if s.name.is_empty() { "this object (give it an instance name to script it)".to_string() } else { format!("\"{}\"", s.name) }),
                        Some(s.on_click.clone()),
                    ),
                    _ => ("Select exactly one object on the stage".into(), None),
                },
                ActTarget::Start => ("Runs once when the movie starts".into(), Some(self.doc.script.clone())),
                ActTarget::EveryFrame => ("Runs every tick at the frame rate (enterFrame)".into(), Some(self.doc.on_frame.clone())),
                ActTarget::Key => ("Runs on each key press; `key` holds the key name".into(), Some(self.doc.on_key.clone())),
                ActTarget::Functions => (
                    "Global functions: `fn name(args) { ... }` here can be called from every script (main timeline, symbols, clicks, keys, timers). Pass values in as arguments; functions can't see outside variables.".into(),
                    Some(self.doc.functions.clone()),
                ),
            };
            ui.label(egui::RichText::new(title).weak());
            if let Some(mut code) = src {
                let before = code.clone();
                let r = egui::ScrollArea::vertical().max_height(ui.available_height() - 150.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut code).code_editor().desired_rows(14).desired_width(f32::INFINITY).lock_focus(true))
                }).inner;
                focused = r.has_focus();
                match rhai::Engine::new().compile(&code) {
                    Ok(_) => ui.colored_label(Color32::from_rgb(110, 210, 120), "✔ syntax OK"),
                    Err(e) => ui.colored_label(Color32::from_rgb(255, 110, 110), format!("✖ {e}")),
                };
                if code != before {
                    changed = true;
                    match self.actions_target {
                        ActTarget::Frame => {
                            let l = &mut self.doc.layers[li];
                            let k = l.key_at(frame);
                            l.keys.get_mut(&k).unwrap().script = code;
                        }
                        ActTarget::Click => {
                            if let Some((sl, s)) = self.sel_shapes().into_iter().next() {
                                for kf in self.doc.layers[sl].keys.values_mut() {
                                    for t in kf.shapes.iter_mut().filter(|t| t.id == s.id) {
                                        t.on_click = code.clone();
                                    }
                                }
                            }
                        }
                        ActTarget::Start => self.doc.script = code,
                        ActTarget::EveryFrame => self.doc.on_frame = code,
                        ActTarget::Key => self.doc.on_key = code,
                        ActTarget::Functions => self.doc.functions = code,
                    }
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Check All Scripts").clicked() {
                    self.check_scripts();
                }
                if ui.button("▶ Test Movie (Ctrl+Enter)").clicked() {
                    self.test_movie();
                }
            });
            if !self.script_errors.is_empty() {
                egui::ScrollArea::vertical().id_salt("errs").max_height(60.0).show(ui, |ui| {
                    for (w, e) in &self.script_errors {
                        ui.colored_label(Color32::from_rgb(255, 110, 110), format!("{w}: {e}"));
                    }
                });
            }
            egui::CollapsingHeader::new("Script reference").show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("help").max_height(220.0).show(ui, |ui| {
                    ui.monospace(runtime::API_HELP);
                });
            });
        });
        if changed {
            self.dirty = true;
            if let Some(s) = snapshot {
                self.undo.push(s);
                self.redo.clear();
                self.script_active = true;
            }
        }
        if !focused {
            self.script_active = false;
        }
        self.show_actions = open;
    }
}

impl FlintApp {
    fn snap(&self, p: P) -> P {
        if self.snap_grid { snap_to(p, self.grid_size) } else { p }
    }

    /// Merges the selected objects (on the active layer's frame) into one group.
    fn group(&mut self) {
        let Some(&(li, _)) = self.sel.first() else { return };
        let ids: Vec<u64> = self.sel.iter().filter(|(l, _)| *l == li).map(|(_, id)| *id).collect();
        if ids.is_empty() {
            return;
        }
        self.checkpoint();
        let frame = self.frame;
        let gid = self.doc.alloc_id();
        let key = self.doc.edit_key(li, frame);
        let mut picked = Vec::new();
        let mut insert_at = 0;
        let mut i = 0;
        key.shapes.retain(|s| {
            let keep = !ids.contains(&s.id);
            if !keep {
                picked.push(s.clone());
                insert_at = i + 1 - picked.len();
            }
            i += 1;
            keep
        });
        let b = picked.iter().map(|s| s.bounds()).reduce(union_bounds).unwrap();
        let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        for s in &mut picked {
            s.xf.x -= c[0];
            s.xf.y -= c[1];
        }
        let n = picked.len();
        let g = Shape { id: gid, geom: Geom::Group { children: picked }, xf: Xform::at(c[0], c[1]), ..Default::default() };
        key.shapes.insert(insert_at.min(key.shapes.len()), g);
        self.sel = vec![(li, gid)];
        self.status = format!("Grouped {n} objects");
    }

    /// Splits selected groups back into their objects, keeping their placement.
    fn ungroup(&mut self) {
        let groups: Vec<(usize, u64)> = self.sel_shapes().into_iter().filter(|(_, s)| s.is_group()).map(|(li, s)| (li, s.id)).collect();
        if groups.is_empty() {
            return;
        }
        self.checkpoint();
        let frame = self.frame;
        let mut new_sel = Vec::new();
        for (li, gid) in groups {
            let key = self.doc.edit_key(li, frame);
            let Some(i) = key.shapes.iter().position(|s| s.id == gid) else { continue };
            let g = key.shapes.remove(i);
            let Geom::Group { children } = g.geom else { continue };
            for (j, mut c) in children.into_iter().enumerate() {
                c.xf = compose(&g.xf, &c.xf);
                new_sel.push((li, c.id));
                key.shapes.insert(i + j, c);
            }
        }
        self.sel = new_sel;
        self.status = "Ungrouped".into();
    }

    /// Moves each selected object onto its own new layer (named after its instance name).
    fn distribute_to_layers(&mut self) {
        let mut shapes = self.sel_shapes();
        if shapes.is_empty() {
            return;
        }
        // bottom-most layers first so earlier inserts never shift the layers still to process
        shapes.sort_by(|a, b| b.0.cmp(&a.0));
        self.checkpoint();
        let frame = self.frame;
        let mut new_sel = Vec::new();
        for (n, (li, s)) in shapes.into_iter().enumerate() {
            let src_len = self.doc.layers[li].len;
            let key_frame = self.doc.layers[li].key_at(frame);
            self.doc.edit_key(li, frame).shapes.retain(|t| t.id != s.id);
            let name = if s.name.is_empty() { format!("{}_{}", shape_kind(&s), s.id) } else { s.name.clone() };
            let mut l = Layer::new(&name, src_len, LAYER_COLORS[(self.doc.layers.len() + n) % LAYER_COLORS.len()]);
            let k = if self.doc.layers[li].keys.contains_key(&frame) { frame } else { key_frame };
            l.keys.insert(k, Keyframe { shapes: vec![s.clone()], ..Default::default() });
            let at = (li + 1).min(self.doc.layers.len());
            self.doc.layers.insert(at, l);
            // indices of layers after `at` shifted; fix the remaining selection
            new_sel = new_sel.into_iter().map(|(l2, id): (usize, u64)| (if l2 >= at { l2 + 1 } else { l2 }, id)).collect();
            new_sel.push((at, s.id));
        }
        self.sel = new_sel;
        self.status = "Distributed to layers".into();
    }
}

pub struct PublishDialog {
    info: hub::PublishInfo,
    update_existing: bool,
    login: hub::LoginForm,
    task: Option<hub::Task<i64>>,
    error: Option<String>,
    published: Option<i64>,
}

impl FlintApp {
    fn open_publish(&mut self) {
        self.finish_text();
        self.script_errors = runtime::check_scripts(&self.doc.normalized());
        let interactive = !self.doc.on_key.trim().is_empty()
            || !self.doc.on_frame.trim().is_empty()
            || self.doc.layers.iter().any(|l| l.keys.values().any(|k| k.shapes.iter().any(|s| !s.on_click.trim().is_empty())));
        let title = self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default();
        self.publish = Some(PublishDialog {
            info: hub::PublishInfo { title, description: String::new(), kind: if interactive { "game" } else { "animation" }.into(), tags: String::new(), update: None },
            update_existing: self.doc.hub_id.is_some(),
            login: hub::LoginForm::default(),
            task: None,
            error: None,
            published: None,
        });
    }

    fn publish_window(&mut self, ctx: &egui::Context) {
        let main_frame = self.main_frame();
        let Some(d) = &mut self.publish else { return };
        if let Some(r) = d.task.as_ref().and_then(|t| t.poll()) {
            d.task = None;
            match r {
                Ok(id) => {
                    d.published = Some(id);
                    if self.doc.hub_id != Some(id) {
                        self.doc.hub_id = Some(id);
                        self.dirty = true;
                    }
                    self.status = format!("Published to Flint Hub (movie #{id})");
                }
                Err(e) => d.error = Some(e),
            }
        }
        let mut open = true;
        let mut start: Option<hub::PublishInfo> = None;
        egui::Window::new("Publish to Flint Hub").collapsible(false).resizable(false).open(&mut open).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            if let Some(id) = d.published {
                ui.heading("🎉 Published!");
                ui.label(format!("\"{}\" is live on Flint Hub as movie #{id}.", d.info.title));
                ui.label("Find it in Flint Player → Flint Hub (it opens there by default).");
                ui.label(egui::RichText::new("Save this document so publishing again updates the same movie.").small().weak());
                return;
            }
            let Some(user) = self.hub.user().map(str::to_string) else {
                ui.label("Log in or create a free account to share your work.");
                ui.separator();
                d.login.ui(ui, &mut self.hub);
                return;
            };
            ui.horizontal(|ui| {
                ui.label(format!("Publishing as {user}"));
                if ui.small_button("Log out").clicked() {
                    self.hub.set_session(None);
                }
            });
            ui.separator();
            egui::Grid::new("pub").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Title");
                ui.add(egui::TextEdit::singleline(&mut d.info.title).desired_width(320.0).char_limit(80));
                ui.end_row();
                ui.label("Type");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.info.kind, "game".to_string(), "🎮 Game");
                    ui.radio_value(&mut d.info.kind, "animation".to_string(), "🎞 Animation");
                });
                ui.end_row();
                ui.label("Description");
                ui.add(egui::TextEdit::multiline(&mut d.info.description).desired_width(320.0).desired_rows(3).char_limit(2000));
                ui.end_row();
                ui.label("Tags");
                ui.add(egui::TextEdit::singleline(&mut d.info.tags).hint_text("space separated").desired_width(320.0).char_limit(200));
                ui.end_row();
                ui.label("Thumbnail");
                ui.label(format!("frame {} (the current frame)", self.frame + 1));
                ui.end_row();
            });
            if let Some(id) = self.doc.hub_id {
                ui.checkbox(&mut d.update_existing, format!("Update my existing movie #{id} instead of posting a new one"));
            }
            if !self.script_errors.is_empty() {
                ui.colored_label(Color32::from_rgb(255, 180, 80), format!("⚠ {} script error(s) — they will show in the player's Output.", self.script_errors.len()));
            }
            if let Some(e) = &d.error {
                ui.colored_label(Color32::from_rgb(255, 110, 110), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let ok = !d.info.title.trim().is_empty() && d.task.is_none();
                if ui.add_enabled(ok, egui::Button::new(egui::RichText::new("🚀 Publish").strong())).clicked() {
                    let mut info = d.info.clone();
                    info.update = if d.update_existing { self.doc.hub_id } else { None };
                    start = Some(info);
                }
                if d.task.is_some() {
                    ui.spinner();
                    ui.label("Uploading…");
                }
            });
        });
        if let Some(info) = start {
            let (h, doc, frame) = (self.hub.clone(), self.doc.normalized(), main_frame);
            d.error = None;
            d.task = Some(hub::Task::spawn(ctx, move || h.publish(&doc, &info, frame)));
        }
        if !open {
            self.publish = None;
        }
    }
}

impl FlintApp {
    /// Frame of the main timeline (the playhead you left when entering a symbol).
    fn main_frame(&self) -> usize {
        self.doc.edit_stack.first().map_or(self.frame, |c| c.frame)
    }

    fn enter_symbol(&mut self, id: u64) {
        if self.doc.edit_stack.iter().any(|c| c.symbol == id) {
            return;
        }
        self.finish_text();
        self.playing = false;
        let ctx = EditCtx { symbol: id, frame: self.frame, layer: self.layer, pan: [self.pan.x, self.pan.y], zoom: self.zoom };
        if self.doc.enter_symbol(ctx) {
            self.frame = 0;
            self.layer = 0;
            self.sel.clear();
            self.pan = Vec2::new(self.doc.width, self.doc.height) * self.zoom * 0.5;
            self.lib_sel = Some(id);
            let name = self.doc.symbol(id).map(|s| s.name.clone()).unwrap_or_default();
            self.status = format!("Editing symbol \"{name}\" — Ctrl+E or double-click empty space to go back");
        }
    }

    fn exit_symbol(&mut self) {
        self.finish_text();
        self.playing = false;
        if let Some(c) = self.doc.exit_symbol() {
            self.frame = c.frame;
            self.layer = c.layer.min(self.doc.layers.len() - 1);
            self.pan = Vec2::new(c.pan[0], c.pan[1]);
            self.zoom = c.zoom;
            self.sel.clear();
            self.status = "Back to parent timeline".into();
        }
    }

    fn breadcrumb(&mut self, ui: &mut Ui) {
        let crumbs: Vec<(u64, String)> = self.doc.edit_stack.iter().map(|c| (c.symbol, c.symbol)).map(|(id, _)| {
            // the symbol being edited has its layers swapped out, but its name is intact
            (id, self.doc.symbol(id).map(|s| s.name.clone()).unwrap_or_default())
        }).collect();
        let mut go_up = 0;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if crumbs.is_empty() {
                ui.label(egui::RichText::new("🎬 Scene 1").strong());
                return;
            }
            if ui.link("🎬 Scene 1").clicked() {
                go_up = crumbs.len();
            }
            for (i, (_, name)) in crumbs.iter().enumerate() {
                ui.label("›");
                let last = i + 1 == crumbs.len();
                if last {
                    ui.label(egui::RichText::new(format!("◆ {name}")).strong().color(Color32::from_rgb(120, 190, 255)));
                } else if ui.link(format!("◆ {name}")).clicked() {
                    go_up = crumbs.len() - 1 - i;
                }
            }
            if ui.button("⬅ Back (Ctrl+E)").clicked() {
                go_up = 1;
            }
        });
        for _ in 0..go_up {
            self.exit_symbol();
        }
    }

    fn unique_symbol_name(&self, base: &str) -> String {
        let mut i = 1;
        loop {
            let n = format!("{base} {i}");
            if !self.doc.symbols.iter().any(|s| s.name == n) {
                return n;
            }
            i += 1;
        }
    }

    /// F8: turns the selection (on one layer) into a new symbol and leaves an instance in its place.
    fn convert_to_symbol(&mut self) {
        let Some(&(li, _)) = self.sel.first() else {
            self.status = "Select something to convert to a symbol".into();
            return;
        };
        let ids: Vec<u64> = self.sel.iter().filter(|(l, _)| *l == li).map(|(_, id)| *id).collect();
        let bounds = self.sel_shapes().iter().filter(|(l, _)| *l == li).map(|(_, s)| s.bounds()).reduce(union_bounds);
        let Some(b) = bounds else { return };
        self.checkpoint();
        let frame = self.frame;
        let sid = self.doc.alloc_id();
        let iid = self.doc.alloc_id();
        let name = self.unique_symbol_name("Symbol");
        let key = self.doc.edit_key(li, frame);
        let mut picked = Vec::new();
        let mut insert_at = 0;
        let mut i = 0;
        key.shapes.retain(|s| {
            let keep = !ids.contains(&s.id);
            if !keep {
                picked.push(s.clone());
                insert_at = i + 1 - picked.len();
            }
            i += 1;
            keep
        });
        let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        for s in &mut picked {
            s.xf.x -= c[0];
            s.xf.y -= c[1];
        }
        let inst = Shape { id: iid, geom: Geom::Symbol { symbol: sid, first: 0, mode: SymMode::Loop, children: Vec::new() }, xf: Xform::at(c[0], c[1]), ..Default::default() };
        key.shapes.insert(insert_at.min(key.shapes.len()), inst);
        let mut layer = Layer::new("Layer_1", 1, LAYER_COLORS[0]);
        layer.keys.get_mut(&0).unwrap().shapes = picked;
        self.doc.symbols.push(Symbol { id: sid, name: name.clone(), layers: vec![layer] });
        self.sel = vec![(li, iid)];
        self.lib_sel = Some(sid);
        self.status = format!("Converted to symbol \"{name}\" — double-click it to animate inside");
    }

    /// Ctrl+F8: makes an empty symbol and opens it for editing.
    fn new_symbol(&mut self) {
        self.checkpoint();
        let sid = self.doc.alloc_id();
        let name = self.unique_symbol_name("Symbol");
        self.doc.symbols.push(Symbol { id: sid, name, layers: vec![Layer::new("Layer_1", 1, LAYER_COLORS[0])] });
        self.enter_symbol(sid);
    }

    fn place_symbol(&mut self, id: u64) {
        let p = self.view.to_stage(self.canvas_center);
        let inst = Shape { geom: Geom::Symbol { symbol: id, first: 0, mode: SymMode::Loop, children: Vec::new() }, xf: Xform::at(p[0], p[1]), ..Default::default() };
        self.add_shape(inst);
    }

    fn library_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("➕ New Symbol").on_hover_text("Ctrl+F8").clicked() {
                self.new_symbol();
            }
            if ui.add_enabled(!self.sel.is_empty(), egui::Button::new("Convert selection (F8)")).clicked() {
                self.convert_to_symbol();
            }
        });
        ui.separator();
        if self.doc.symbols.is_empty() {
            ui.label(egui::RichText::new("No symbols yet. Select a drawing and press F8 to turn it into a reusable symbol with its own timeline — e.g. a character with idle/run/jump animations.").weak());
            return;
        }
        let editing: Vec<u64> = self.doc.edit_stack.iter().map(|c| c.symbol).collect();
        let list: Vec<(u64, String, usize)> = self.doc.symbols.iter().map(|s| (s.id, s.name.clone(), if editing.contains(&s.id) { 0 } else { s.frame_count() })).collect();
        egui::ScrollArea::vertical().max_height(220.0).id_salt("liblist").show(ui, |ui| {
            for (id, name, frames) in &list {
                let label = if editing.contains(id) { format!("◆ {name}   (editing)") } else { format!("◆ {name}   {frames}f") };
                let r = ui.selectable_label(self.lib_sel == Some(*id), label);
                if r.clicked() {
                    self.lib_sel = Some(*id);
                }
                if r.double_clicked() {
                    self.enter_symbol(*id);
                }
            }
        });
        let Some(id) = self.lib_sel.filter(|id| list.iter().any(|(i, _, _)| i == id)) else { return };
        ui.separator();
        // preview (frame 1)
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(260.0), 150.0), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, c32(self.doc.background));
        if editing.contains(&id) {
            p.text(rect.center(), Align2::CENTER_CENTER, "being edited", FontId::proportional(13.0), Color32::from_gray(120));
        } else {
            let shapes = self.doc.symbol_shapes(id, 0);
            if let Some(b) = shapes.iter().map(|s| s.bounds()).reduce(union_bounds) {
                let (w, h) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
                let zoom = ((rect.width() - 16.0) / w).min((rect.height() - 16.0) / h).min(4.0);
                let origin = rect.center() - Vec2::new((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5) * zoom;
                let view = View { origin, zoom };
                for s in &shapes {
                    render::draw_shape(&p, &view, s, &render::Style { alpha: 1.0, outline: None });
                }
            } else {
                p.text(rect.center(), Align2::CENTER_CENTER, "(empty)", FontId::proportional(13.0), Color32::from_gray(120));
            }
        }
        let mut name = self.doc.symbol(id).map(|s| s.name.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui.text_edit_singleline(&mut name).changed() && !name.trim().is_empty() {
                if let Some(s) = self.doc.symbol_mut(id) {
                    s.name = name.clone();
                }
                self.dirty = true;
            }
        });
        let labels: Vec<String> = self.doc.symbol(id).map(|s| s.layers.iter().flat_map(|l| l.keys.values().filter(|k| !k.label.is_empty()).map(|k| k.label.clone())).collect()).unwrap_or_default();
        if !labels.is_empty() {
            ui.label(egui::RichText::new(format!("Labels: {}", labels.join(", "))).small());
        }
        ui.horizontal_wrapped(|ui| {
            let can_place = !editing.contains(&id) && self.layer_editable(self.layer);
            if ui.add_enabled(can_place, egui::Button::new("Place on Stage")).clicked() {
                self.place_symbol(id);
            }
            if ui.add_enabled(!editing.contains(&id), egui::Button::new("✏ Edit")).clicked() {
                self.enter_symbol(id);
            }
            if ui.add_enabled(!editing.contains(&id), egui::Button::new("Duplicate")).clicked() {
                self.checkpoint();
                let nid = self.doc.alloc_id();
                let mut copy = self.doc.symbol(id).unwrap().clone();
                copy.id = nid;
                copy.name = format!("{} copy", copy.name);
                self.doc.symbols.push(copy);
                self.lib_sel = Some(nid);
            }
            // every timeline is in doc.layers or doc.symbols, swapped or not
            let in_use = self.doc.symbol_in_use(id);
            if ui.add_enabled(!in_use && !editing.contains(&id), egui::Button::new("🗑 Delete")).on_disabled_hover_text("Used on a timeline").clicked() {
                self.checkpoint();
                self.doc.symbols.retain(|s| s.id != id);
                self.lib_sel = None;
            }
        });
        ui.label(egui::RichText::new("Tip: give instances names in Properties, put frame labels (idle, run, jump) inside the symbol, then script hero.goto_and_play(\"run\").").small().weak());
    }
}

impl FlintApp {
    /// Paint Bucket: recolor a filled shape, fill a lone closed outline, or fill the region enclosed by lines.
    fn bucket_click(&mut self, sp: P) {
        let frame = self.frame;
        let tol = 2.0 / self.zoom;
        let fc = rgba(self.fill_col);
        let editable: Vec<usize> = (0..self.doc.layers.len()).filter(|&li| self.layer_editable(li)).collect();

        // 1. a filled shape (or text) under the cursor: recolor it
        let filled = editable.iter().find_map(|&li| {
            self.doc.layer_shapes(li, frame).into_iter().rev().find_map(|top| {
                let leaves = top.flatten();
                leaves.iter().rposition(|l| (l.fill.is_some() && l.encloses(sp)) || (l.is_text() && l.hit(sp, tol))).map(|leaf| (li, top, leaf))
            })
        });
        if let Some((li, top, leaf)) = filled {
            self.paint_leaf(li, &top, leaf, |m| m.fill = Some(fc));
            return;
        }
        if !self.layer_editable(self.layer) {
            self.status = "Active layer is locked or hidden".into();
            return;
        }
        let active: Vec<Shape> = self.doc.layer_shapes(self.layer, frame);
        let leaves: Vec<Shape> = active.iter().flat_map(|s| s.flatten()).collect();

        // 2. a single closed outline with nothing inside it: give the shape itself a fill
        for top in active.iter().rev() {
            let tl = top.flatten();
            let Some(leaf) = tl.iter().rposition(|l| l.encloses(sp)) else { continue };
            let outline = &tl[leaf];
            let others_inside = leaves.iter().any(|o| {
                if o.id == outline.id {
                    return false;
                }
                let b = o.bounds();
                outline.encloses([(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5])
            });
            if !others_inside {
                self.paint_leaf(self.layer, top, leaf, |m| m.fill = Some(fc));
                return;
            }
            break;
        }

        // 3. the region enclosed by lines: this layer's first, then everything visible
        let mut result = crate::fill::region_fill(&leaves, sp, self.zoom, self.bucket_gap);
        if matches!(result, Err(crate::fill::FillError::NotEnclosed)) {
            let all: Vec<Shape> = self.doc.render_list(frame).into_iter().flat_map(|(_, s)| s.flatten()).collect();
            if all.len() > leaves.len() {
                result = crate::fill::region_fill(&all, sp, self.zoom, self.bucket_gap);
            }
        }
        match result {
            Ok(poly) => {
                self.checkpoint();
                let b = bounds_of(&poly);
                let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
                let pts = poly.iter().map(|p| [p[0] - c[0], p[1] - c[1]]).collect();
                let id = self.doc.alloc_id();
                let shape = Shape { id, geom: Geom::Path { pts, closed: true }, xf: Xform::at(c[0], c[1]), fill: Some(fc), stroke: None, ..Default::default() };
                // fills sit under the lines that bound them
                self.doc.edit_key(self.layer, frame).shapes.insert(0, shape);
                self.status = "Filled".into();
            }
            Err(crate::fill::FillError::NotEnclosed) => {
                self.status = "That area isn't enclosed — close the gap or pick a bigger Gap in the Paint Bucket options".into();
            }
            Err(crate::fill::FillError::OnShape) => {
                self.status = "Click inside an area enclosed by lines".into();
            }
        }
    }

    /// Applies `f` to leaf number `leaf` (in `flatten()` order) of the top-level shape `top`.
    fn paint_leaf(&mut self, li: usize, top: &Shape, leaf: usize, mut f: impl FnMut(&mut Shape)) {
        if top.symbol_id().is_some() {
            self.status = "That's a symbol instance — double-click it to paint inside".into();
            return;
        }
        self.checkpoint();
        let frame = self.frame;
        if let Some(m) = self.doc.shape_mut(li, frame, top.id) {
            let mut i = 0;
            m.for_leaves_mut(&mut |m| {
                if i == leaf {
                    f(m);
                }
                i += 1;
            });
        }
    }
}
