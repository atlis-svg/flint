//! Flint Player: plays published `.flx` movies (or `.flint` documents), runs their scripts,
//! and browses Flint Hub. A projector is this executable with a movie appended; it plays that
//! movie on launch and hides everything else.

// No console window for the GUI apps on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use egui::{Color32, FontId, Key, Rect, RichText, Sense, Ui, Vec2};
use flint::hub::{self, Comment, Hub, LoginForm, MovieInfo, Task};
use flint::render::{self, View, c32};
use flint::runtime::Runtime;
use flint::{flx, model::Document};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const ACCENT: Color32 = Color32::from_rgb(255, 120, 60);

enum Thumb {
    Loading(Task<Vec<u8>>),
    Ready(egui::TextureHandle),
    Missing,
}

struct Gallery {
    kind: &'static str,
    sort: &'static str,
    query: String,
    mine: bool,
    page: usize,
    movies: Vec<MovieInfo>,
    more: bool,
    task: Option<Task<(Vec<MovieInfo>, bool)>>,
    error: Option<String>,
}

/// The Hub movie currently open, with its social bits.
struct Showing {
    info: MovieInfo,
    comments: Vec<Comment>,
    comments_task: Option<Task<Vec<Comment>>>,
    post_task: Option<Task<()>>,
    like_task: Option<Task<(bool, i64)>>,
    draft: String,
    show_comments: bool,
}

struct Player {
    rt: Option<Runtime>,
    name: String,
    error: Option<String>,
    output: Vec<String>,
    show_output: bool,
    projector: bool,
    title: String,
    hub: Hub,
    in_hub: bool,
    gallery: Gallery,
    thumbs: HashMap<i64, Thumb>,
    showing: Option<Showing>,
    download: Option<(MovieInfo, Task<Document>)>,
    login: Option<LoginForm>,
    status: String,
    show_about: bool,
    intro: Option<flint::intro::Intro>,
}

impl Player {
    fn load_doc(&mut self, doc: Document, name: String) {
        self.rt = Some(Runtime::new(doc));
        self.name = name;
        self.error = None;
        self.output.clear();
        self.in_hub = false;
    }

    fn open_path(&mut self, p: PathBuf) {
        match flx::load(&p) {
            Ok(doc) => {
                self.showing = None;
                self.load_doc(doc, p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Flint movie", &["flx", "flint"]).pick_file() {
            self.open_path(p);
        }
    }

    fn refresh(&mut self, ctx: &egui::Context, append: bool) {
        let g = &mut self.gallery;
        if !append {
            g.page = 0;
        }
        let (h, kind, sort, q, page) = (self.hub.clone(), g.kind, g.sort, g.query.clone(), g.page);
        let author = if g.mine { self.hub.user().unwrap_or("").to_string() } else { String::new() };
        g.error = None;
        g.task = Some(Task::spawn(ctx, move || h.list(kind, sort, &q, &author, page)));
        if !append {
            g.movies.clear();
        }
    }

    fn open_hub_movie(&mut self, ctx: &egui::Context, m: MovieInfo) {
        let h = self.hub.clone();
        let id = m.id;
        self.download = Some((m, Task::spawn(ctx, move || h.download(id, true))));
    }

    fn back_to_hub(&mut self, ctx: &egui::Context) {
        self.rt = None;
        self.showing = None;
        self.name.clear();
        self.in_hub = true;
        if self.gallery.movies.is_empty() {
            self.refresh(ctx, false);
        }
    }

    fn poll_tasks(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.gallery.task.as_ref().and_then(|t| t.poll()) {
            self.gallery.task = None;
            match r {
                Ok((ms, more)) => {
                    self.gallery.movies.extend(ms);
                    self.gallery.more = more;
                }
                Err(e) => self.gallery.error = Some(e),
            }
        }
        if let Some(r) = self.download.as_ref().and_then(|(_, t)| t.poll()) {
            let (info, _) = self.download.take().unwrap();
            match r {
                Ok(doc) => {
                    let title = info.title.clone();
                    let h = self.hub.clone();
                    let id = info.id;
                    self.showing = Some(Showing {
                        info,
                        comments: Vec::new(),
                        comments_task: Some(Task::spawn(ctx, move || h.comments(id))),
                        post_task: None,
                        like_task: None,
                        draft: String::new(),
                        show_comments: false,
                    });
                    self.load_doc(doc, title);
                }
                Err(e) => self.status = format!("Couldn't open movie: {e}"),
            }
        }
        if let Some(s) = &mut self.showing {
            if let Some(r) = s.comments_task.as_ref().and_then(|t| t.poll()) {
                s.comments_task = None;
                match r {
                    Ok(c) => {
                        s.info.comments = c.len() as i64;
                        s.comments = c;
                    }
                    Err(e) => self.status = e,
                }
            }
            if let Some(r) = s.like_task.as_ref().and_then(|t| t.poll()) {
                s.like_task = None;
                match r {
                    Ok((liked, likes)) => {
                        s.info.liked = liked;
                        s.info.likes = likes;
                        if let Some(m) = self.gallery.movies.iter_mut().find(|m| m.id == s.info.id) {
                            m.likes = likes;
                        }
                    }
                    Err(e) => self.status = e,
                }
            }
            if let Some(r) = s.post_task.as_ref().and_then(|t| t.poll()) {
                s.post_task = None;
                match r {
                    Ok(()) => {
                        s.draft.clear();
                        let (h, id) = (self.hub.clone(), s.info.id);
                        s.comments_task = Some(Task::spawn(ctx, move || h.comments(id)));
                    }
                    Err(e) => self.status = e,
                }
            }
        }
        for (id, t) in self.thumbs.iter_mut() {
            if let Thumb::Loading(task) = t {
                if let Some(r) = task.poll() {
                    *t = match r.ok().and_then(|png| tiny_skia::Pixmap::decode_png(&png).ok()) {
                        Some(pm) => {
                            let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                            Thumb::Ready(ctx.load_texture(format!("thumb{id}"), img, egui::TextureOptions::LINEAR))
                        }
                        None => Thumb::Missing,
                    };
                }
            }
        }
    }

    fn remix(&mut self) {
        let Some(rt) = &self.rt else { return };
        let mut doc = rt.doc().clone();
        doc.hub_id = None; // a remix is a new movie, not an update of the original
        let Some(editor) = std::env::current_exe().ok().map(|p| p.with_file_name(if cfg!(windows) { "flint.exe" } else { "flint" })).filter(|p| p.exists()) else {
            self.status = "Flint editor not found next to the player".into();
            return;
        };
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let dir = home.map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Documents").join("Flint");
        let safe: String = self.name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let path = dir.join(format!("{}-remix.flint", if safe.is_empty() { "movie" } else { &safe }));
        let res = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|_| serde_json::to_vec(&doc).map_err(|e| e.to_string()))
            .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
            .and_then(|_| std::process::Command::new(editor).arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string()));
        self.status = match res {
            Ok(()) => format!("Opened a copy in Flint: {}", path.display()),
            Err(e) => format!("Remix failed: {e}"),
        };
    }
}

impl eframe::App for Player {
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
        self.poll_tasks(&ctx);
        let title = if self.name.is_empty() {
            if self.in_hub { "Flint Hub".to_string() } else { "Flint Player".to_string() }
        } else if self.projector {
            self.name.clone()
        } else {
            format!("{} - Flint Player", self.name)
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if let Some(f) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
            self.open_path(f);
        }
        if !self.projector {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::O)) {
                self.open_dialog();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::H)) {
                self.back_to_hub(&ctx);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F12)) {
            self.show_output = !self.show_output;
        }
        if let Some(rt) = &mut self.rt {
            for line in rt.take_log() {
                println!("{line}");
                self.output.push(line);
            }
            if self.output.len() > 2000 {
                self.output.drain(..self.output.len() - 2000);
            }
        }

        self.login_window(&ctx);
        flint::about::window(&ctx, &mut self.show_about, "Flint Player");

        if self.rt.is_none() && !self.projector {
            if !self.in_hub {
                self.in_hub = true;
                self.refresh(&ctx, false);
            }
            self.hub_screen(ui);
            return;
        }

        if self.showing.is_some() {
            self.movie_bar(ui);
        }
        if self.show_output {
            egui::Panel::bottom("output").resizable(true).default_size(140.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Output");
                    if ui.small_button("Clear").clicked() {
                        self.output.clear();
                    }
                });
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                    for l in &self.output {
                        ui.monospace(l);
                    }
                });
            });
        }
        if self.showing.as_ref().is_some_and(|s| s.show_comments) {
            egui::Panel::right("comments").resizable(true).default_size(300.0).show(ui, |ui| self.comments_panel(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| self.stage(ui));
    }
}

impl Player {
    fn login_window(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.login else { return };
        let mut open = true;
        let mut done = false;
        egui::Window::new("Flint Hub account").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            done = form.ui(ui, &mut self.hub);
        });
        if done {
            self.status = format!("Logged in as {}", self.hub.user().unwrap_or(""));
        }
        if !open || done {
            self.login = None;
        }
    }

    fn account_button(&mut self, ui: &mut Ui) {
        match self.hub.user().map(str::to_string) {
            Some(u) => {
                ui.menu_button(format!("👤 {u}"), |ui| {
                    if ui.button("My movies").clicked() {
                        self.gallery.mine = true;
                        self.gallery.query.clear();
                        self.refresh(&ui.ctx().clone(), false);
                        ui.close();
                    }
                    if ui.button("Log out").clicked() {
                        self.hub.set_session(None);
                        self.gallery.mine = false;
                        ui.close();
                    }
                });
            }
            None => {
                if ui.button("Log in / Sign up").clicked() {
                    self.login = Some(LoginForm::default());
                }
            }
        }
    }

    fn hub_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("hubbar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔥 Flint Hub").size(20.0).strong().color(ACCENT));
                ui.separator();
                let mut changed = false;
                for (k, lbl) in [("", "All"), ("game", "🎮 Games"), ("animation", "🎞 Animations")] {
                    if ui.selectable_label(self.gallery.kind == k && !self.gallery.mine, lbl).clicked() {
                        self.gallery.kind = k;
                        self.gallery.mine = false;
                        changed = true;
                    }
                }
                ui.separator();
                let sort_label = match self.gallery.sort {
                    "top" => "Most liked",
                    "plays" => "Most played",
                    _ => "Newest",
                };
                egui::ComboBox::from_id_salt("sort").selected_text(sort_label).show_ui(ui, |ui| {
                    for (k, l) in [("new", "Newest"), ("top", "Most liked"), ("plays", "Most played")] {
                        changed |= ui.selectable_value(&mut self.gallery.sort, k, l).changed();
                    }
                });
                let r = ui.add(egui::TextEdit::singleline(&mut self.gallery.query).hint_text("🔍 Search").desired_width(180.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    changed = true;
                }
                if ui.button("⟳").on_hover_text("Refresh").clicked() {
                    changed = true;
                }
                if changed {
                    self.refresh(&ctx, false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("ℹ").on_hover_text("About & licenses").clicked() {
                        self.show_about = true;
                    }
                    self.account_button(ui);
                    if ui.button("📂 Open file…").clicked() {
                        self.open_dialog();
                    }
                });
            });
            ui.add_space(4.0);
        });
        egui::Panel::bottom("hubstatus").exact_size(22.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some((m, _)) = &self.download {
                    ui.spinner();
                    ui.label(format!("Loading \"{}\"…", m.title));
                } else {
                    ui.label(RichText::new(&self.status).small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(&self.hub.base).small().weak());
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 110, 110), e);
            }
            if let Some(e) = &self.gallery.error {
                ui.colored_label(Color32::from_rgb(255, 110, 110), e);
            }
            if self.gallery.mine {
                ui.heading("My movies");
            }
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let card = Vec2::new(232.0, 236.0);
                let per_row = ((ui.available_width() + 12.0) / (card.x + 12.0)).floor().max(1.0) as usize;
                let mut clicked = None;
                let movies = self.gallery.movies.clone();
                for row in movies.chunks(per_row) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        for m in row {
                            if self.card(ui, m, card) {
                                clicked = Some(m.clone());
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
                if self.gallery.task.is_some() {
                    ui.vertical_centered(|ui| ui.spinner());
                } else if self.gallery.movies.is_empty() && self.gallery.error.is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new("Nothing here yet.").size(18.0));
                        ui.label("Make something in Flint and use File → Publish to Flint Hub.");
                    });
                } else if self.gallery.more && ui.button("Load more").clicked() {
                    self.gallery.page += 1;
                    self.refresh(&ctx, true);
                }
                if let Some(m) = clicked {
                    if self.download.is_none() {
                        self.open_hub_movie(&ctx, m);
                    }
                }
            });
        });
    }

    fn card(&mut self, ui: &mut Ui, m: &MovieInfo, size: Vec2) -> bool {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = ui.painter();
        let hovered = resp.hovered();
        p.rect_filled(rect, 8.0, if hovered { Color32::from_gray(58) } else { Color32::from_gray(44) });
        let thumb_rect = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(size.x - 12.0, 165.0));
        p.rect_filled(thumb_rect, 4.0, Color32::from_gray(20));
        let t = self.thumbs.entry(m.id).or_insert_with(|| {
            let (h, id) = (self.hub.clone(), m.id);
            Thumb::Loading(Task::spawn(ui.ctx(), move || h.thumb(id)))
        });
        match t {
            Thumb::Ready(tex) => {
                let s = tex.size_vec2();
                let k = (thumb_rect.width() / s.x).min(thumb_rect.height() / s.y);
                let r = Rect::from_center_size(thumb_rect.center(), s * k);
                p.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            Thumb::Loading(_) => {
                p.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "…", FontId::proportional(20.0), Color32::from_gray(120));
            }
            Thumb::Missing => {
                p.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "🎞", FontId::proportional(32.0), Color32::from_gray(90));
            }
        }
        if hovered {
            p.circle_filled(thumb_rect.center(), 26.0, Color32::from_black_alpha(150));
            p.text(thumb_rect.center() + Vec2::new(2.0, 0.0), egui::Align2::CENTER_CENTER, "▶", FontId::proportional(24.0), Color32::WHITE);
        }
        let badge = if m.kind == "game" { ("GAME", Color32::from_rgb(70, 140, 255)) } else { ("ANIM", Color32::from_rgb(200, 90, 220)) };
        let br = Rect::from_min_size(thumb_rect.min + Vec2::splat(6.0), Vec2::new(40.0, 16.0));
        p.rect_filled(br, 3.0, badge.1);
        p.text(br.center(), egui::Align2::CENTER_CENTER, badge.0, FontId::proportional(10.0), Color32::WHITE);
        let mut title = m.title.clone();
        if title.chars().count() > 28 {
            title = title.chars().take(27).collect::<String>() + "…";
        }
        let y = thumb_rect.bottom() + 8.0;
        p.text(egui::pos2(rect.left() + 8.0, y), egui::Align2::LEFT_TOP, title, FontId::proportional(15.0), Color32::WHITE);
        p.text(egui::pos2(rect.left() + 8.0, y + 20.0), egui::Align2::LEFT_TOP, format!("by {} · {}", m.author, hub::ago(m.created)), FontId::proportional(11.0), Color32::from_gray(170));
        p.text(
            egui::pos2(rect.left() + 8.0, y + 38.0),
            egui::Align2::LEFT_TOP,
            format!("♥ {}   ▶ {}   💬 {}", m.likes, m.plays, m.comments),
            FontId::proportional(11.0),
            Color32::from_gray(200),
        );
        let resp = resp.on_hover_text(if m.description.is_empty() { m.title.clone() } else { format!("{}\n\n{}", m.title, m.description) });
        resp.clicked()
    }

    fn movie_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut back = false;
        let mut remix = false;
        let logged_in = self.hub.user().is_some();
        let mut want_login = false;
        egui::Panel::top("moviebar").show(ui, |ui| {
            let Some(s) = &mut self.showing else { return };
            ui.horizontal(|ui| {
                if ui.button("← Hub").clicked() {
                    back = true;
                }
                ui.separator();
                ui.label(RichText::new(&s.info.title).strong());
                ui.label(RichText::new(format!("by {}", s.info.author)).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🔀 Remix in Flint").on_hover_text("Open an editable copy in the Flint editor").clicked() {
                        remix = true;
                    }
                    if ui.selectable_label(s.show_comments, format!("💬 {}", s.info.comments)).clicked() {
                        s.show_comments = !s.show_comments;
                    }
                    let heart = if s.info.liked { RichText::new(format!("♥ {}", s.info.likes)).color(ACCENT) } else { RichText::new(format!("♡ {}", s.info.likes)) };
                    if ui.add_enabled(s.like_task.is_none(), egui::Button::new(heart)).clicked() {
                        if logged_in {
                            let (h, id) = (self.hub.clone(), s.info.id);
                            s.like_task = Some(Task::spawn(&ctx, move || h.like(id)));
                        } else {
                            want_login = true;
                        }
                    }
                    ui.label(RichText::new(format!("▶ {}", s.info.plays + 1)).weak());
                    if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).small().weak());
                    }
                });
            });
        });
        if want_login {
            self.login = Some(LoginForm::default());
        }
        if remix {
            self.remix();
        }
        if back {
            self.back_to_hub(&ctx);
        }
    }

    fn comments_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let logged_in = self.hub.user().is_some();
        let mut want_login = false;
        let Some(s) = &mut self.showing else { return };
        ui.heading("Comments");
        if !s.info.description.is_empty() {
            ui.label(&s.info.description);
        }
        if !s.info.tags.is_empty() {
            ui.label(RichText::new(s.info.tags.split_whitespace().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")).small().color(ACCENT));
        }
        ui.separator();
        if logged_in {
            ui.add(egui::TextEdit::multiline(&mut s.draft).hint_text("Say something nice…").desired_rows(2).desired_width(f32::INFINITY));
            if ui.add_enabled(s.post_task.is_none() && !s.draft.trim().is_empty(), egui::Button::new("Post")).clicked() {
                let (h, id, body) = (self.hub.clone(), s.info.id, s.draft.clone());
                s.post_task = Some(Task::spawn(&ctx, move || h.comment(id, &body)));
            }
        } else if ui.button("Log in to comment").clicked() {
            want_login = true;
        }
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if s.comments_task.is_some() && s.comments.is_empty() {
                ui.spinner();
            }
            for c in &s.comments {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.author).strong());
                    ui.label(RichText::new(hub::ago(c.created)).small().weak());
                });
                ui.label(&c.body);
                ui.add_space(6.0);
            }
            if s.comments.is_empty() && s.comments_task.is_none() {
                ui.label(RichText::new("No comments yet.").weak());
            }
        });
        if want_login {
            self.login = Some(LoginForm::default());
        }
    }

    fn stage(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click());
        let ctx = ui.ctx().clone();
        let Some(rt) = &mut self.rt else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(rect.height() * 0.4);
                    ui.colored_label(Color32::from_rgb(255, 110, 110), self.error.as_deref().unwrap_or("No movie"));
                });
            });
            return;
        };
        let (w, h) = {
            let d = rt.doc();
            (d.width, d.height)
        };
        let zoom = (rect.width() / w).min(rect.height() / h).max(0.01);
        let size = Vec2::new(w, h) * zoom;
        let origin = rect.center() - size * 0.5;
        let view = View { origin, zoom };
        let stage = Rect::from_min_size(origin, size);

        // input (only when no text field has focus, so typing a comment doesn't drive the game)
        let typing = ctx.egui_wants_keyboard_input();
        let (hover, events, keys) = ctx.input(|i| (i.pointer.hover_pos(), i.events.clone(), i.keys_down.iter().map(|k| k.name().to_string()).collect::<HashSet<_>>()));
        let mouse = hover.map(|p| view.to_stage(p)).unwrap_or([0.0, 0.0]);
        rt.set_input(mouse, if typing { HashSet::new() } else { keys });
        if !typing {
            for ev in &events {
                if let egui::Event::Key { key, pressed: true, repeat: false, .. } = ev {
                    if *key != Key::F12 {
                        rt.key_pressed(key.name());
                    }
                }
            }
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                rt.click(view.to_stage(p));
            }
        }
        if hover.is_some_and(|p| stage.contains(p) && rt.hover_clickable(view.to_stage(p))) {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let dt = ctx.input(|i| i.stable_dt) as f64;
        rt.tick(dt);
        ctx.request_repaint();

        let painter = ui.painter_at(stage);
        painter.rect_filled(stage, 0.0, c32(rt.doc().background));
        let style = render::Style { alpha: 1.0, outline: None };
        for (_, s) in rt.shapes() {
            render::draw_shape(&painter, &view, &s, &style);
        }

        // Flash-style right-click menu
        let projector = self.projector;
        let mut open = false;
        let mut to_hub = false;
        resp.context_menu(|ui| {
            let playing = rt.playing();
            if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                rt.set_playing(!playing);
                ui.close();
            }
            if ui.button("Rewind").clicked() {
                rt.restart();
                ui.close();
            }
            if ui.button("Forward").clicked() {
                rt.step(true);
                ui.close();
            }
            if ui.button("Back").clicked() {
                rt.step(false);
                ui.close();
            }
            let mut l = rt.looping();
            if ui.checkbox(&mut l, "Loop").changed() {
                rt.set_looping(l);
            }
            ui.separator();
            if ui.checkbox(&mut self.show_output, "Show Output (F12)").clicked() {
                ui.close();
            }
            if !projector {
                if ui.button("Browse Flint Hub  Ctrl+H").clicked() {
                    to_hub = true;
                    ui.close();
                }
                if ui.button("Open…").clicked() {
                    open = true;
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("About Flint Player…").clicked() {
                self.show_about = true;
                ui.close();
            }
        });
        if open {
            self.open_dialog();
        }
        if to_hub {
            self.back_to_hub(&ctx);
        }
    }
}

fn main() -> eframe::Result {
    let mut player = Player {
        rt: None,
        name: String::new(),
        error: None,
        output: Vec::new(),
        show_output: false,
        projector: false,
        title: String::new(),
        hub: Hub::load(),
        in_hub: false,
        gallery: Gallery { kind: "", sort: "new", query: String::new(), mine: false, page: 0, movies: Vec::new(), more: false, task: None, error: None },
        thumbs: HashMap::new(),
        showing: None,
        download: None,
        login: None,
        status: String::new(),
        show_about: false,
        intro: None,
    };
    let mut size = [1100.0f32, 760.0];
    if let Some(m) = std::env::current_exe().ok().and_then(|p| flx::embedded_in_file(&p)) {
        player.projector = true;
        match m {
            Ok(doc) => {
                let name = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
                player.load_doc(doc, name);
            }
            Err(e) => player.error = Some(e),
        }
    } else if let Some(arg) = std::env::args().nth(1) {
        player.open_path(PathBuf::from(arg));
    }
    // the intro plays when the player opens to the Hub, not for a movie or a projector
    if player.rt.is_none() && !player.projector {
        player.intro = flint::intro::Intro::new();
    }
    if let Some(rt) = &player.rt {
        let d = rt.doc();
        let k = (1600.0 / d.width).min(1000.0 / d.height).min(1.0);
        size = [d.width * k, d.height * k];
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Flint Player").with_inner_size(size).with_min_inner_size([320.0, 240.0]).with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Flint Player", opts, Box::new(|_| Ok(Box::new(player))))
}
