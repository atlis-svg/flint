//! Flint Hub client: accounts, publishing, browsing, likes and comments.
//! Calls are blocking; UIs run them on a thread through [`Task`].

use crate::{export, flx, model::Document};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::mpsc;

pub const DEFAULT_HUB: &str = "https://flint-hub.v0rt3xv1sta.workers.dev";

pub fn base_url() -> String {
    std::env::var("FLINT_HUB").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_HUB.to_string()).trim_end_matches('/').to_string()
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct MovieInfo {
    pub id: i64,
    pub author: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub tags: String,
    pub created: f64,
    pub plays: i64,
    pub likes: i64,
    pub size: i64,
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    pub frames: i64,
    #[serde(default)]
    pub comments: i64,
    #[serde(default)]
    pub liked: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Comment {
    pub id: i64,
    pub author: String,
    pub body: String,
    pub created: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub name: String,
    pub token: String,
}

#[derive(Deserialize)]
struct Page {
    movies: Vec<MovieInfo>,
    more: bool,
}

#[derive(Clone, Default)]
pub struct PublishInfo {
    pub title: String,
    pub description: String,
    pub kind: String, // "game" | "animation"
    pub tags: String,
    /// Update this existing movie (must be yours) instead of creating a new one.
    pub update: Option<i64>,
}

#[derive(Clone)]
pub struct Hub {
    pub base: String,
    pub session: Option<Session>,
    agent: ureq::Agent,
}

fn session_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("flint").join("hub.json"))
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            #[derive(Deserialize)]
            struct E {
                error: String,
            }
            resp.into_json::<E>().map(|e| e.error).unwrap_or_else(|_| format!("server returned {code}"))
        }
        ureq::Error::Transport(t) => format!("can't reach Flint Hub: {t}"),
    }
}

impl Hub {
    /// Hub client with the saved login (if any).
    pub fn load() -> Self {
        let session = session_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok());
        Hub { base: base_url(), session, agent: ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(30)).build() }
    }

    pub fn user(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.name.as_str())
    }

    pub fn set_session(&mut self, s: Option<Session>) {
        self.session = s;
        if let Some(p) = session_path() {
            match &self.session {
                Some(s) => {
                    let _ = std::fs::create_dir_all(p.parent().unwrap());
                    let _ = std::fs::write(&p, serde_json::to_vec(s).unwrap_or_default());
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                    }
                }
                None => {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    pub fn movie_url(&self, id: i64) -> String {
        format!("{}/api/movies/{id}", self.base)
    }

    fn req(&self, method: &str, path: &str) -> ureq::Request {
        let r = self.agent.request(method, &format!("{}/api{path}", self.base));
        match &self.session {
            Some(s) => r.set("Authorization", &format!("Bearer {}", s.token)),
            None => r,
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.req("GET", path).call().map_err(http_err)?.into_json().map_err(|e| e.to_string())
    }

    fn post_json<T: serde::de::DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T, String> {
        self.req("POST", path).send_json(body).map_err(http_err)?.into_json().map_err(|e| e.to_string())
    }

    fn get_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let resp = self.req("GET", path).call().map_err(http_err)?;
        let mut out = Vec::new();
        resp.into_reader().take(8_000_000).read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    pub fn register(&self, name: &str, password: &str) -> Result<Session, String> {
        self.post_json("/register", serde_json::json!({ "name": name, "password": password }))
    }

    pub fn login(&self, name: &str, password: &str) -> Result<Session, String> {
        self.post_json("/login", serde_json::json!({ "name": name, "password": password }))
    }

    /// kind: "", "game" or "animation"; sort: "new", "top" or "plays".
    pub fn list(&self, kind: &str, sort: &str, query: &str, author: &str, page: usize) -> Result<(Vec<MovieInfo>, bool), String> {
        let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
        let p: Page = self.get_json(&format!("/movies?kind={}&sort={}&q={}&author={}&page={page}", enc(kind), enc(sort), enc(query), enc(author)))?;
        Ok((p.movies, p.more))
    }

    pub fn movie(&self, id: i64) -> Result<MovieInfo, String> {
        self.get_json(&format!("/movies/{id}"))
    }

    /// Downloads and decodes a movie. `count_play` bumps its play counter.
    pub fn download(&self, id: i64, count_play: bool) -> Result<Document, String> {
        let bytes = self.get_bytes(&format!("/movies/{id}/file?play={}", if count_play { 1 } else { 0 }))?;
        flx::decode(&bytes)
    }

    pub fn thumb(&self, id: i64) -> Result<Vec<u8>, String> {
        self.get_bytes(&format!("/movies/{id}/thumb"))
    }

    pub fn like(&self, id: i64) -> Result<(bool, i64), String> {
        #[derive(Deserialize)]
        struct L {
            liked: bool,
            likes: i64,
        }
        let l: L = self.post_json(&format!("/movies/{id}/like"), serde_json::json!({}))?;
        Ok((l.liked, l.likes))
    }

    pub fn comments(&self, id: i64) -> Result<Vec<Comment>, String> {
        #[derive(Deserialize)]
        struct C {
            comments: Vec<Comment>,
        }
        Ok(self.get_json::<C>(&format!("/movies/{id}/comments"))?.comments)
    }

    pub fn comment(&self, id: i64, body: &str) -> Result<(), String> {
        self.post_json::<serde_json::Value>(&format!("/movies/{id}/comments"), serde_json::json!({ "body": body })).map(|_| ())
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.req("DELETE", &format!("/movies/{id}")).call().map_err(http_err).map(|_| ())
    }

    /// Publishes `doc` with a thumbnail of `thumb_frame`. Returns the movie id.
    pub fn publish(&self, doc: &Document, info: &PublishInfo, thumb_frame: usize) -> Result<i64, String> {
        let file = flx::encode(doc)?;
        let scale = (320.0 / doc.width).min(240.0 / doc.height).min(1.0);
        let thumb = export::render_frame(doc, thumb_frame.min(doc.frame_count() - 1), scale).encode_png().map_err(|e| e.to_string())?;
        let b64 = base64::engine::general_purpose::STANDARD;
        #[derive(Deserialize)]
        struct R {
            id: i64,
        }
        let r: R = self.post_json(
            "/movies",
            serde_json::json!({
                "id": info.update,
                "title": info.title,
                "description": info.description,
                "kind": info.kind,
                "tags": info.tags,
                "width": doc.width,
                "height": doc.height,
                "fps": doc.fps,
                "frames": doc.frame_count(),
                "file": b64.encode(&file),
                "thumb": b64.encode(&thumb),
            }),
        )?;
        Ok(r.id)
    }
}

/// A background request whose result the UI polls each frame.
pub struct Task<T>(mpsc::Receiver<Result<T, String>>);

impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
        Task(rx)
    }

    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.0.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("request failed".into())),
        }
    }
}

pub fn ago(ms: f64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
    let s = ((now - ms) / 1000.0).max(0.0) as u64;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        86400..2_592_000 => format!("{}d ago", s / 86400),
        _ => format!("{}mo ago", s / 2_592_000),
    }
}

/// Log in / sign up form shared by the editor and the player.
#[derive(Default)]
pub struct LoginForm {
    pub name: String,
    pub password: String,
    pub register: bool,
    pub error: Option<String>,
    task: Option<Task<Session>>,
}

impl LoginForm {
    /// Draws the form; returns true once the user is logged in.
    pub fn ui(&mut self, ui: &mut egui::Ui, hub: &mut Hub) -> bool {
        if let Some(r) = self.task.as_ref().and_then(|t| t.poll()) {
            self.task = None;
            match r {
                Ok(s) => {
                    hub.set_session(Some(s));
                    self.password.clear();
                    self.error = None;
                    return true;
                }
                Err(e) => self.error = Some(e),
            }
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.register, false, "Log in");
            ui.selectable_value(&mut self.register, true, "Create account");
        });
        let mut submit = false;
        egui::Grid::new("login").num_columns(2).show(ui, |ui| {
            ui.label("Username");
            ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(180.0));
            ui.end_row();
            ui.label("Password");
            let r = ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(180.0));
            submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
        });
        if self.register {
            ui.label(egui::RichText::new("3-24 letters, numbers, _ or -; password 6+ characters").small().weak());
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(255, 110, 110), e);
        }
        let busy = self.task.is_some();
        ui.horizontal(|ui| {
            submit |= ui.add_enabled(!busy, egui::Button::new(if self.register { "Create account" } else { "Log in" })).clicked();
            if busy {
                ui.spinner();
            }
        });
        if submit && !busy {
            let (h, n, p, reg) = (hub.clone(), self.name.trim().to_string(), self.password.clone(), self.register);
            self.task = Some(Task::spawn(ui.ctx(), move || if reg { h.register(&n, &p) } else { h.login(&n, &p) }));
        }
        false
    }
}
