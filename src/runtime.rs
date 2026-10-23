//! Movie runtime: timeline playback plus Rhai scripting (Flint's ActionScript).

use crate::model::*;
use rhai::{AST, Dynamic, Engine, EvalAltResult, FnPtr, ImmutableString, Scope};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Script-driven property overrides for a named object.
#[derive(Clone, Default)]
struct Ovr {
    x: Option<f32>,
    y: Option<f32>,
    rot: Option<f32>,
    sx: Option<f32>,
    sy: Option<f32>,
    alpha: Option<f32>,
    visible: Option<bool>,
    fill: Option<Rgba>,
    stroke: Option<Rgba>,
    text: Option<String>,
}

struct State {
    doc: Document,
    frame: usize,
    playing: bool,
    looping: bool,
    goto: Option<(usize, bool)>,
    base: Vec<(usize, Shape)>,
    ovr: HashMap<String, Ovr>,
    mouse: P,
    keys: HashSet<String>,
    log: Vec<String>,
    time: f64,
    rng: u64,
    timers: Vec<Timer>,
    next_timer: i64,
    cancelled: HashSet<i64>,
    current_timer: Option<i64>,
    /// Playheads of symbol instances on stage, by instance id.
    clips: HashMap<u64, Clip>,
}

struct Clip {
    symbol: u64,
    mode: SymMode,
    frame: usize,
    playing: bool,
    /// Frame whose scripts already ran.
    ran: Option<usize>,
    /// Skip one step after a goto so the target frame is shown and its script runs.
    hold: bool,
}

/// Script key of a top-level object: its instance name, or `#id` when unnamed.
fn obj_key(s: &Shape) -> String {
    if s.name.is_empty() { format!("#{}", s.id) } else { s.name.clone() }
}

enum Job {
    /// The rest of a script after a top-level `sleep(...)` line.
    Rest { label: String, src: String },
    /// A closure passed to `after` / `every`.
    Call(FnPtr),
}

struct Timer {
    id: i64,
    at: f64,
    every: Option<f64>,
    job: Job,
}

impl State {
    fn cancelled_now(&mut self, id: i64) -> bool {
        self.cancelled.remove(&id)
    }

    fn total(&self) -> usize {
        self.doc.frame_count()
    }

    fn apply(&self, s: &mut Shape) -> bool {
        let Some(o) = self.ovr.get(&obj_key(s)) else { return true };
        if let Some(v) = o.x {
            s.xf.x = v;
        }
        if let Some(v) = o.y {
            s.xf.y = v;
        }
        if let Some(v) = o.rot {
            s.xf.rot = v;
        }
        if let Some(v) = o.sx {
            s.xf.sx = v;
        }
        if let Some(v) = o.sy {
            s.xf.sy = v;
        }
        if o.fill.is_some() || o.stroke.is_some() || o.alpha.is_some() {
            s.for_leaves_mut(&mut |s| {
                if let Some(c) = o.fill {
                    if s.fill.is_some() || s.is_text() {
                        s.fill = Some(c);
                    }
                }
                if let Some(c) = o.stroke {
                    s.stroke = Some(c);
                }
                if let Some(a) = o.alpha {
                    for c in [&mut s.fill, &mut s.stroke].into_iter().flatten() {
                        c[3] = (c[3] as f32 * a.clamp(0.0, 1.0)) as u8;
                    }
                }
            });
        }
        if let (Some(t), Geom::Text { text, .. }) = (&o.text, &mut s.geom) {
            *text = t.clone();
        }
        o.visible != Some(false)
    }

    fn shapes(&self) -> Vec<(usize, Shape)> {
        let mut out = Vec::with_capacity(self.base.len());
        for (li, s) in &self.base {
            let mut s = s.clone();
            if self.apply(&mut s) {
                out.push((*li, s));
            }
        }
        out
    }

    fn find(&self, name: &str) -> Option<Shape> {
        let mut s = self.base.iter().rev().find(|(_, s)| obj_key(s) == name)?.1.clone();
        self.apply(&mut s);
        Some(s)
    }

    fn resolve_frame(&self, target: &Dynamic) -> Option<usize> {
        if let Ok(n) = target.as_int() {
            return Some((n.max(1) as usize - 1).min(self.total() - 1));
        }
        if let Ok(f) = target.as_float() {
            return Some((f.max(1.0) as usize - 1).min(self.total() - 1));
        }
        let label = target.clone().into_string().ok()?;
        self.doc.layers.iter().find_map(|l| l.keys.iter().find(|(_, k)| k.label == label).map(|(f, _)| *f))
    }

    /// Runs `f` on the playhead of the symbol instance with script key `key`.
    fn with_clip<R>(&mut self, key: &str, f: impl FnOnce(&mut Clip, &Symbol) -> R) -> Option<R> {
        let id = self.base.iter().find(|(_, s)| obj_key(s) == key)?.1.id;
        let State { clips, doc, .. } = self;
        let clip = clips.get_mut(&id)?;
        let sym = doc.symbol(clip.symbol)?;
        Some(f(clip, sym))
    }

    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Handle to a named stage object inside scripts: `obj("ball").x += 5`.
#[derive(Clone)]
pub struct Obj {
    name: ImmutableString,
    st: Rc<RefCell<State>>,
}

impl Obj {
    fn get(&self) -> Option<Shape> {
        self.st.borrow().find(&self.name)
    }
    fn set(&self, f: impl FnOnce(&mut Ovr)) {
        let mut st = self.st.borrow_mut();
        f(st.ovr.entry(self.name.to_string()).or_default());
    }
    fn bounds(&self) -> Option<[f32; 4]> {
        self.get().map(|s| s.bounds())
    }
}

fn num(v: &Dynamic) -> f32 {
    v.as_float().map(|f| f as f32).or_else(|_| v.as_int().map(|i| i as f32)).unwrap_or(0.0)
}

pub fn parse_color(s: &str) -> Option<Rgba> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([v(0)?, v(2)?, v(4)?, 255]),
        8 => Some([v(0)?, v(2)?, v(4)?, v(6)?]),
        _ => None,
    }
}

fn color_str(c: Option<Rgba>) -> String {
    c.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).unwrap_or_default()
}

fn build_engine(st: &Rc<RefCell<State>>) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(2_000_000);
    e.set_max_call_levels(64);
    {
        let st = st.clone();
        e.on_print(move |s| st.borrow_mut().log.push(s.to_string()));
    }
    {
        let st = st.clone();
        e.on_debug(move |s, _, _| st.borrow_mut().log.push(s.to_string()));
    }

    macro_rules! f {
        ($name:expr, |$st:ident $(, $a:ident : $t:ty)*| $body:expr) => {{
            let s = st.clone();
            e.register_fn($name, move |$($a: $t),*| { let $st = &s; $body });
        }};
    }

    // --- timeline control
    f!("stop", |s| s.borrow_mut().playing = false);
    f!("play", |s| s.borrow_mut().playing = true);
    f!("goto_and_play", |s, t: Dynamic| {
        let mut st = s.borrow_mut();
        if let Some(f) = st.resolve_frame(&t) {
            st.goto = Some((f, true));
        }
    });
    f!("goto_and_stop", |s, t: Dynamic| {
        let mut st = s.borrow_mut();
        if let Some(f) = st.resolve_frame(&t) {
            st.goto = Some((f, false));
        }
    });
    f!("next_frame", |s| {
        let mut st = s.borrow_mut();
        let f = (st.frame + 1).min(st.total() - 1);
        st.goto = Some((f, false));
    });
    f!("prev_frame", |s| {
        let mut st = s.borrow_mut();
        let f = st.frame.saturating_sub(1);
        st.goto = Some((f, false));
    });
    f!("current_frame", |s| s.borrow().frame as i64 + 1);
    f!("total_frames", |s| s.borrow().total() as i64);
    f!("is_playing", |s| s.borrow().playing);

    // --- output, input, misc
    f!("trace", |s, v: Dynamic| s.borrow_mut().log.push(v.to_string()));
    f!("mouse_x", |s| s.borrow().mouse[0] as f64);
    f!("mouse_y", |s| s.borrow().mouse[1] as f64);
    f!("key_down", |s, k: ImmutableString| s.borrow().keys.contains(k.as_str()));
    f!("random", |s| s.borrow_mut().random());
    f!("random_int", |s, a: i64, b: i64| {
        let r = s.borrow_mut().random();
        let (lo, hi) = (a.min(b), a.max(b));
        lo + (r * (hi - lo + 1) as f64) as i64
    });
    f!("time", |s| s.borrow().time);

    // --- timers
    fn schedule(st: &Rc<RefCell<State>>, secs: f32, every: bool, f: FnPtr) -> i64 {
        let mut st = st.borrow_mut();
        let id = st.next_timer;
        st.next_timer += 1;
        let secs = secs.max(0.0) as f64;
        let at = st.time + secs;
        st.timers.push(Timer { id, at, every: every.then_some(secs.max(0.001)), job: Job::Call(f) });
        id
    }
    f!("after", |s, secs: Dynamic, f: FnPtr| schedule(s, num(&secs), false, f));
    f!("every", |s, secs: Dynamic, f: FnPtr| schedule(s, num(&secs), true, f));
    f!("cancel", |s, id: i64| {
        let mut st = s.borrow_mut();
        st.timers.retain(|t| t.id != id);
        st.cancelled.insert(id); // in case it's the timer currently running
    });
    f!("cancel_all", |s| s.borrow_mut().timers.clear());
    f!("stop_timer", |s| {
        let mut st = s.borrow_mut();
        if let Some(id) = st.current_timer {
            st.cancelled.insert(id);
        }
    });
    // Overrides Rhai's built-in blocking sleep (exact INT/FLOAT matches beat the package version).
    const NESTED_SLEEP: &str = "sleep() only works on its own line outside { } blocks; inside if/for/fn use after(seconds, || { ... })";
    for name in ["sleep", "wait"] {
        f!(name, |s, _secs: i64| s.borrow_mut().log.push(NESTED_SLEEP.into()));
        f!(name, |s, _secs: f64| s.borrow_mut().log.push(NESTED_SLEEP.into()));
        f!(name, |s, _secs: Dynamic| s.borrow_mut().log.push(NESTED_SLEEP.into()));
    }
    f!("stage_width", |s| s.borrow().doc.width as f64);
    f!("stage_height", |s| s.borrow().doc.height as f64);
    f!("exists", |s, n: ImmutableString| s.borrow().find(&n).is_some());
    f!("reset", |s, n: ImmutableString| {
        s.borrow_mut().ovr.remove(n.as_str());
    });

    // --- objects
    e.register_type_with_name::<Obj>("Obj");
    f!("obj", |s, n: ImmutableString| Obj { name: n, st: s.clone() });

    macro_rules! prop {
        ($name:expr, |$o:ident| $get:expr, |$o2:ident, $v:ident| $set:expr) => {
            e.register_get($name, |$o: &mut Obj| -> f64 { $get });
            e.register_set($name, |$o2: &mut Obj, $v: Dynamic| { $set });
        };
    }
    prop!("x", |o| o.get().map_or(0.0, |s| s.xf.x as f64), |o, v| o.set(|ov| ov.x = Some(num(&v))));
    prop!("y", |o| o.get().map_or(0.0, |s| s.xf.y as f64), |o, v| o.set(|ov| ov.y = Some(num(&v))));
    prop!("rotation", |o| o.get().map_or(0.0, |s| s.xf.rot.to_degrees() as f64), |o, v| o.set(|ov| ov.rot = Some(num(&v).to_radians())));
    prop!("scale_x", |o| o.get().map_or(1.0, |s| s.xf.sx as f64), |o, v| o.set(|ov| ov.sx = Some(num(&v))));
    prop!("scale_y", |o| o.get().map_or(1.0, |s| s.xf.sy as f64), |o, v| o.set(|ov| ov.sy = Some(num(&v))));
    prop!(
        "alpha",
        |o| o.st.borrow().ovr.get(o.name.as_str()).and_then(|ov| ov.alpha).unwrap_or(1.0) as f64,
        |o, v| o.set(|ov| ov.alpha = Some(num(&v)))
    );
    e.register_get_set(
        "visible",
        |o: &mut Obj| o.st.borrow().ovr.get(o.name.as_str()).and_then(|ov| ov.visible).unwrap_or(true),
        |o: &mut Obj, v: bool| o.set(|ov| ov.visible = Some(v)),
    );
    e.register_get(
        "fill",
        |o: &mut Obj| color_str(o.get().and_then(|s| s.fill)),
    );
    e.register_set(
        "fill",
        |o: &mut Obj, v: ImmutableString| o.set(|ov| ov.fill = parse_color(&v)),
    );
    e.register_get(
        "stroke",
        |o: &mut Obj| color_str(o.get().and_then(|s| s.stroke)),
    );
    e.register_set(
        "stroke",
        |o: &mut Obj, v: ImmutableString| o.set(|ov| ov.stroke = parse_color(&v)),
    );
    e.register_get(
        "text",
        |o: &mut Obj| match o.get().map(|s| s.geom) {
            Some(Geom::Text { text, .. }) => text,
            _ => String::new(),
        },
    );
    e.register_set(
        "text",
        |o: &mut Obj, v: Dynamic| o.set(|ov| ov.text = Some(v.to_string())),
    );
    e.register_get("name", |o: &mut Obj| o.name.to_string());
    e.register_get("width", |o: &mut Obj| o.bounds().map_or(0.0, |b| (b[2] - b[0]) as f64));
    e.register_get("height", |o: &mut Obj| o.bounds().map_or(0.0, |b| (b[3] - b[1]) as f64));
    e.register_fn("hit_test", |a: &mut Obj, b: Obj| match (a.bounds(), b.bounds()) {
        (Some(p), Some(q)) => p[0] < q[2] && q[0] < p[2] && p[1] < q[3] && q[1] < p[3],
        _ => false,
    });
    e.register_fn("hit_point", |a: &mut Obj, x: Dynamic, y: Dynamic| a.get().is_some_and(|s| s.hit([num(&x), num(&y)], 0.0)));
    e.register_fn("to_string", |o: &mut Obj| format!("obj(\"{}\")", o.name));

    // --- symbol instances (movie clips) have their own timeline
    fn clip_frame(sym: &Symbol, t: &Dynamic) -> Option<usize> {
        let n = sym.frame_count();
        if let Ok(i) = t.as_int() {
            return Some((i.max(1) as usize - 1).min(n - 1));
        }
        if let Ok(f) = t.as_float() {
            return Some((f.max(1.0) as usize - 1).min(n - 1));
        }
        sym.label_frame(&t.clone().into_string().ok()?)
    }
    fn goto(o: &Obj, t: &Dynamic, play: bool) {
        let name = o.name.to_string();
        let found = o.st.borrow_mut().with_clip(&name, |c, sym| match clip_frame(sym, t) {
            Some(f) => {
                c.frame = f;
                c.playing = play;
                c.ran = None;
                c.hold = true;
                true
            }
            None => false,
        });
        if found != Some(true) {
            o.st.borrow_mut().log.push(format!("{}.goto: no symbol instance or frame/label {t}", o.name));
        }
    }
    e.register_fn("play", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut().with_clip(&n, |c, _| c.playing = true);
    });
    e.register_fn("stop", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut().with_clip(&n, |c, _| c.playing = false);
    });
    e.register_fn("goto_and_play", |o: &mut Obj, t: Dynamic| goto(o, &t, true));
    e.register_fn("goto_and_stop", |o: &mut Obj, t: Dynamic| goto(o, &t, false));
    e.register_fn("next_frame", |o: &mut Obj| {
        let n = o.name.to_string();
        let f = o.st.borrow_mut().with_clip(&n, |c, _| c.frame + 2).unwrap_or(1);
        goto(o, &Dynamic::from(f as i64), false);
    });
    e.register_fn("prev_frame", |o: &mut Obj| {
        let n = o.name.to_string();
        let f = o.st.borrow_mut().with_clip(&n, |c, _| c.frame).unwrap_or(1).max(1);
        goto(o, &Dynamic::from(f as i64), false);
    });
    e.register_get("current_frame", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut().with_clip(&n, |c, _| c.frame as i64 + 1).unwrap_or(1)
    });
    e.register_get("total_frames", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut().with_clip(&n, |_, s| s.frame_count() as i64).unwrap_or(1)
    });
    e.register_get("playing", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut().with_clip(&n, |c, _| c.playing).unwrap_or(false)
    });
    e.register_get("current_label", |o: &mut Obj| {
        let n = o.name.to_string();
        o.st.borrow_mut()
            .with_clip(&n, |c, s| s.layers.iter().filter_map(|l| l.keys.range(..=c.frame).rev().find(|(_, k)| !k.label.is_empty()).map(|(f, k)| (*f, k.label.clone()))).max_by_key(|(f, _)| *f).map(|(_, l)| l))
            .flatten()
            .unwrap_or_default()
    });
    e
}

/// Compile-checks every script in the document. Returns (location, error) pairs.
pub fn check_scripts(doc: &Document) -> Vec<(String, String)> {
    let e = Engine::new();
    let mut errs = Vec::new();
    for (where_, src) in script_sources(doc) {
        if let Err(err) = e.compile(&src) {
            errs.push((where_, err.to_string()));
        }
    }
    errs
}

pub fn script_sources(doc: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (lbl, s) in [("Movie: functions", &doc.functions), ("Movie: on start", &doc.script), ("Movie: every frame", &doc.on_frame), ("Movie: on key", &doc.on_key)] {
        if !s.trim().is_empty() {
            out.push((lbl.to_string(), s.clone()));
        }
    }
    for l in &doc.layers {
        for (f, k) in &l.keys {
            if !k.script.trim().is_empty() {
                out.push((format!("{} frame {}", l.name, f + 1), k.script.clone()));
            }
            for s in &k.shapes {
                if !s.on_click.trim().is_empty() {
                    let n = if s.name.is_empty() { format!("#{}", s.id) } else { s.name.clone() };
                    out.push((format!("{} frame {} \"{}\" on click", l.name, f + 1, n), s.on_click.clone()));
                }
            }
        }
    }
    for sym in &doc.symbols {
        for l in &sym.layers {
            for (f, k) in &l.keys {
                if !k.script.trim().is_empty() {
                    out.push((format!("Symbol \"{}\" {} frame {}", sym.name, l.name, f + 1), k.script.clone()));
                }
            }
        }
    }
    out
}

pub struct Runtime {
    st: Rc<RefCell<State>>,
    engine: Engine,
    scope: Scope<'static>,
    cache: HashMap<String, Result<AST, String>>,
    /// Functions (incl. closures) from every compiled script, for calling timers later.
    lib: AST,
    /// Global functions (Movie: functions + functions in Movie: on start), linked into every script.
    fn_lib: AST,
    accum: f64,
}

impl Runtime {
    pub fn new(doc: Document) -> Self {
        let doc = doc.normalized();
        let st = Rc::new(RefCell::new(State {
            doc,
            frame: 0,
            playing: true,
            looping: true,
            goto: None,
            base: Vec::new(),
            ovr: HashMap::new(),
            mouse: [0.0, 0.0],
            keys: HashSet::new(),
            log: Vec::new(),
            time: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64),
            timers: Vec::new(),
            next_timer: 1,
            cancelled: HashSet::new(),
            current_timer: None,
            clips: HashMap::new(),
        }));
        let engine = build_engine(&st);
        let mut rt = Runtime { st, engine, scope: Scope::new(), cache: HashMap::new(), lib: AST::empty(), fn_lib: AST::empty(), accum: 0.0 };
        rt.build_fn_lib();
        rt.start();
        rt
    }

    fn start(&mut self) {
        let init = self.st.borrow().doc.script.clone();
        self.refresh();
        self.run("on start", &init);
        let pending = self.st.borrow_mut().goto.take();
        match pending {
            Some((f, play)) => {
                self.st.borrow_mut().playing = play;
                self.enter(f);
            }
            None => self.enter(0),
        }
    }

    pub fn restart(&mut self) {
        let (doc, looping) = {
            let st = self.st.borrow();
            (st.doc.clone(), st.looping)
        };
        *self = Runtime::new(doc);
        self.st.borrow_mut().looping = looping;
    }

    pub fn doc(&self) -> std::cell::Ref<'_, Document> {
        std::cell::Ref::map(self.st.borrow(), |s| &s.doc)
    }

    fn build_fn_lib(&mut self) {
        let (funcs, init) = {
            let st = self.st.borrow();
            (st.doc.functions.clone(), st.doc.script.clone())
        };
        let mut lib = AST::empty();
        for (label, src) in [("functions", funcs), ("on start", init)] {
            if src.trim().is_empty() {
                continue;
            }
            // only definitions are taken; `sleep` lines don't matter here
            let src = split_sleep(&src).map(|(a, _, _)| a).unwrap_or(src);
            match self.engine.compile(&src) {
                Ok(ast) => lib = lib.merge(&ast.clone_functions_only()),
                Err(e) if label == "functions" => self.st.borrow_mut().log.push(format!("[functions] syntax error: {e}")),
                Err(_) => {}
            }
        }
        self.lib = lib.clone();
        self.fn_lib = lib;
    }

    fn refresh(&mut self) {
        let mut guard = self.st.borrow_mut();
        let State { doc, clips, base, frame, .. } = &mut *guard;
        *base = doc.render_list_with(*frame, &|s, def| clips.get(&s.id).map_or(def, |c| c.frame));
        // instances that just appeared start their own playhead; ones that left are forgotten
        let mut fresh = false;
        let mut present = HashSet::new();
        for (_, s) in base.iter() {
            if let Geom::Symbol { symbol, first, mode, .. } = s.geom {
                present.insert(s.id);
                clips.entry(s.id).or_insert_with(|| {
                    fresh = true;
                    let len = doc.symbol(symbol).map_or(1, |x| x.frame_count());
                    Clip { symbol, mode, frame: first.min(len - 1), playing: mode != SymMode::Single, ran: None, hold: true }
                });
            }
        }
        clips.retain(|id, _| present.contains(id));
        if fresh {
            *base = doc.render_list_with(*frame, &|s, def| clips.get(&s.id).map_or(def, |c| c.frame));
        }
    }

    /// Steps every playing movie clip one frame and runs symbol frame scripts (with `me` = the instance).
    fn step_clips(&mut self) {
        let scripts: Vec<(String, String, String)> = {
            let mut guard = self.st.borrow_mut();
            let State { doc, clips, base, .. } = &mut *guard;
            let mut out = Vec::new();
            for (id, c) in clips.iter_mut() {
                let Some(sym) = doc.symbol(c.symbol) else { continue };
                let len = sym.frame_count();
                if c.hold {
                    c.hold = false;
                } else if c.playing {
                    if c.frame + 1 < len {
                        c.frame += 1;
                    } else if c.mode == SymMode::Loop {
                        c.frame = 0;
                    } else {
                        c.playing = false;
                    }
                }
                if c.ran != Some(c.frame) {
                    c.ran = Some(c.frame);
                    let Some(inst) = base.iter().find(|(_, s)| s.id == *id) else { continue };
                    for l in sym.layers.iter().rev() {
                        if let Some(k) = l.keys.get(&c.frame).filter(|k| !k.script.trim().is_empty() && c.frame < l.len) {
                            out.push((obj_key(&inst.1), format!("{} frame {}", sym.name, c.frame + 1), k.script.clone()));
                        }
                    }
                }
            }
            out
        };
        for (key, label, src) in scripts {
            let me = Obj { name: key.into(), st: self.st.clone() };
            self.scope.set_value("me", me);
            self.run(&label, &src);
        }
        self.refresh();
    }

    fn run(&mut self, label: &str, src: &str) {
        if src.trim().is_empty() {
            return;
        }
        if let Some((first, secs, rest)) = split_sleep(src) {
            self.run(label, &first);
            let secs = match self.engine.eval_expression_with_scope::<Dynamic>(&mut self.scope, &secs) {
                Ok(v) => num(&v).max(0.0) as f64,
                Err(e) => {
                    self.st.borrow_mut().log.push(format!("[{label}] sleep({secs}): {e}"));
                    0.0
                }
            };
            let mut st = self.st.borrow_mut();
            let id = st.next_timer;
            st.next_timer += 1;
            let at = st.time + secs;
            st.timers.push(Timer { id, at, every: None, job: Job::Rest { label: label.to_string(), src: rest } });
            return;
        }
        if !self.cache.contains_key(src) {
            let compiled = self.engine.compile(src).map(|ast| self.fn_lib.merge(&ast)).map_err(|e| e.to_string());
            if let Ok(ast) = &compiled {
                self.lib = self.lib.merge(&ast.clone_functions_only());
            }
            self.cache.insert(src.to_string(), compiled);
        }
        let ast = self.cache.get(src).unwrap();
        let res: Result<(), Box<EvalAltResult>> = match ast {
            Ok(ast) => self.engine.run_ast_with_scope(&mut self.scope, ast),
            Err(e) => {
                let msg = format!("[{label}] syntax error: {e}");
                let mut st = self.st.borrow_mut();
                if st.log.last() != Some(&msg) {
                    st.log.push(msg);
                }
                return;
            }
        };
        if let Err(e) = res {
            self.st.borrow_mut().log.push(format!("[{label}] error: {e}"));
        }
    }

    /// Moves the playhead to `f` and runs the frame scripts there, following script gotos.
    fn enter(&mut self, f: usize) {
        let mut target = Some(f);
        let mut guard = 0;
        while let Some(f) = target.take() {
            guard += 1;
            if guard > 64 {
                self.st.borrow_mut().log.push("goto loop detected; stopping".into());
                break;
            }
            let scripts: Vec<(String, String)> = {
                let mut st = self.st.borrow_mut();
                st.frame = f.min(st.total() - 1);
                let fr = st.frame;
                st.doc
                    .layers
                    .iter()
                    .rev()
                    .filter(|l| fr < l.len)
                    .filter_map(|l| l.keys.get(&fr).filter(|k| !k.script.trim().is_empty()).map(|k| (format!("{} frame {}", l.name, fr + 1), k.script.clone())))
                    .collect()
            };
            self.refresh();
            for (label, src) in scripts {
                self.run(&label, &src);
                if self.st.borrow().goto.is_some() {
                    break;
                }
            }
            let pending = self.st.borrow_mut().goto.take();
            if let Some((g, play)) = pending {
                self.st.borrow_mut().playing = play;
                target = Some(g);
            }
        }
    }

    fn flush_goto(&mut self) {
        let pending = self.st.borrow_mut().goto.take();
        if let Some((f, play)) = pending {
            self.st.borrow_mut().playing = play;
            self.enter(f);
        }
    }

    /// Advances time; runs frame scripts and the every-frame handler at the movie's frame rate.
    pub fn tick(&mut self, dt: f64) {
        let fps = self.st.borrow().doc.fps.max(1.0) as f64;
        self.accum += dt.min(0.25);
        self.st.borrow_mut().time += dt;
        let step = 1.0 / fps;
        let mut n = 0;
        while self.accum >= step {
            self.accum -= step;
            n += 1;
            if n > 4 {
                self.accum = 0.0;
                break;
            }
            self.advance();
        }
        self.run_timers();
    }

    fn run_timers(&mut self) {
        let now = self.st.borrow().time;
        let mut due: Vec<Timer> = {
            let mut st = self.st.borrow_mut();
            let (due, keep) = std::mem::take(&mut st.timers).into_iter().partition(|t| t.at <= now);
            st.timers = keep;
            due
        };
        due.sort_by(|a, b| a.at.total_cmp(&b.at));
        for mut t in due {
            match &t.job {
                Job::Rest { label, src } => {
                    let (label, src) = (label.clone(), src.clone());
                    self.run(&label, &src);
                }
                Job::Call(f) => {
                    self.st.borrow_mut().current_timer = Some(t.id);
                    let res = f.call::<Dynamic>(&self.engine, &self.lib, ());
                    self.st.borrow_mut().current_timer = None;
                    if let Err(e) = res {
                        self.st.borrow_mut().log.push(format!("[timer] error: {e}"));
                    }
                }
            }
            if let Some(every) = t.every {
                // only reschedule if the closure didn't cancel itself
                let mut st = self.st.borrow_mut();
                t.at = (t.at + every).max(now);
                if !st.timers.iter().any(|x| x.id == t.id) && !st.cancelled_now(t.id) {
                    st.timers.push(t);
                }
            }
            self.flush_goto();
        }
    }

    fn advance(&mut self) {
        let (playing, frame, total, looping) = {
            let st = self.st.borrow();
            (st.playing, st.frame, st.total(), st.looping)
        };
        if playing {
            if frame + 1 < total {
                self.enter(frame + 1);
            } else if looping {
                self.enter(0);
            } else {
                self.st.borrow_mut().playing = false;
            }
        }
        self.step_clips();
        let src = self.st.borrow().doc.on_frame.clone();
        self.run("every frame", &src);
        self.flush_goto();
    }

    pub fn set_input(&mut self, mouse: P, keys: HashSet<String>) {
        let mut st = self.st.borrow_mut();
        st.mouse = mouse;
        st.keys = keys;
    }

    fn clickable_at(&self, p: P) -> Option<Shape> {
        self.st.borrow().shapes().into_iter().rev().map(|(_, s)| s).find(|s| !s.on_click.trim().is_empty() && s.hit(p, 2.0))
    }

    pub fn hover_clickable(&self, p: P) -> bool {
        self.clickable_at(p).is_some()
    }

    pub fn click(&mut self, p: P) {
        let Some(s) = self.clickable_at(p) else { return };
        let me = Obj { name: obj_key(&s).into(), st: self.st.clone() };
        self.scope.set_value("me", me);
        let label = if s.name.is_empty() { "click".to_string() } else { format!("{} on click", s.name) };
        self.run(&label, &s.on_click);
        self.flush_goto();
    }

    pub fn key_pressed(&mut self, key: &str) {
        let src = self.st.borrow().doc.on_key.clone();
        if src.trim().is_empty() {
            return;
        }
        self.scope.set_value("key", key.to_string());
        self.run("on key", &src);
        self.flush_goto();
    }

    pub fn shapes(&self) -> Vec<(usize, Shape)> {
        self.st.borrow().shapes()
    }

    pub fn frame(&self) -> usize {
        self.st.borrow().frame
    }
    pub fn playing(&self) -> bool {
        self.st.borrow().playing
    }
    pub fn set_playing(&mut self, p: bool) {
        self.st.borrow_mut().playing = p;
    }
    pub fn looping(&self) -> bool {
        self.st.borrow().looping
    }
    pub fn set_looping(&mut self, l: bool) {
        self.st.borrow_mut().looping = l;
    }
    pub fn step(&mut self, forward: bool) {
        let (f, total) = {
            let st = self.st.borrow();
            (st.frame, st.total())
        };
        self.st.borrow_mut().playing = false;
        let nf = if forward { (f + 1).min(total - 1) } else { f.saturating_sub(1) };
        self.enter(nf);
    }
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.st.borrow_mut().log)
    }
}

pub const API_HELP: &str = "\
Flint scripts are Rhai (rhai.rs): let x = 1; if/else, while, for i in 0..10, fn, arrays, maps.
Variables persist across all scripts in a movie (frames, clicks, keys).

Where scripts run
  Movie: on start      once, before frame 1
  Movie: every frame   every tick at the frame rate, even while stopped
  Movie: on key        on each key press; `key` holds its name (\"Left\", \"Space\", \"A\", …)
  Frame script         when the playhead enters that keyframe
  Object on click      when the object is clicked; `me` is that object

Timeline
  stop()  play()  goto_and_play(frame_or_label)  goto_and_stop(frame_or_label)
  next_frame()  prev_frame()  current_frame()  total_frames()  is_playing()
  (frames are numbered from 1, like the timeline)

Global functions — write them in \"Movie: functions\" (or on start); every script can call them:
  fn clamp(v, lo, hi) { if v < lo { lo } else if v > hi { hi } else { v } }
  fn face(o, dir) { o.scale_x = dir; }      // functions can't see outside variables:
  face(obj(\"hero\"), -1);                   // pass objects/values in, return results out

Symbols (movie clips) — F8 converts a selection; each instance plays its own timeline:
  hero.play()  hero.stop()  hero.goto_and_play(\"run\")  hero.goto_and_stop(1)
  hero.next_frame()  hero.prev_frame()  hero.current_frame  hero.total_frames
  hero.playing  hero.current_label
  Frame scripts inside a symbol get `me` = that instance:  me.stop();  me.goto_and_play(\"idle\");
  Unnamed objects can still be `me`; named ones are reachable everywhere with obj(\"name\").

Objects — give a shape an instance name in Properties, then:
  let ball = obj(\"ball\");   (store it in a variable before assigning: ball.x = 5)
  ball.x  ball.y  ball.rotation (deg)  ball.scale_x  ball.scale_y  ball.alpha (0..1)
  ball.visible  ball.fill = \"#ff0000\"  ball.stroke  ball.text  ball.width  ball.height
  ball.hit_test(obj(\"wall\"))  ball.hit_point(x, y)  exists(\"name\")  reset(\"name\")

Waiting (the movie keeps running while you wait)
  sleep(2);                         on its own line: the rest of the script runs 2 s later
  after(1.5, || { trace(\"hi\"); });  run a block later (works anywhere, even inside if)
  let t = every(0.5, || { n += 1; });  repeat a block;  cancel(t) stops it;  cancel_all()
  stop_timer()                      inside an every() block: stop repeating

Input & misc
  mouse_x()  mouse_y()  key_down(\"Left\")  random()  random_int(1, 6)  time()
  stage_width()  stage_height()  trace(value)  print(value)
  math: sin cos sqrt abs floor round min max PI() …
";

/// Splits `src` at the first top-level `sleep(...)` / `wait(...)` statement.
/// Returns (code before, seconds expression, code after).
fn split_sleep(src: &str) -> Option<(String, String, String)> {
    let b = src.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' | b'`' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            _ if depth == 0 => {
                for kw in ["sleep", "wait"] {
                    let boundary = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.');
                    if boundary && src[i..].starts_with(kw) {
                        let mut j = i + kw.len();
                        while j < b.len() && b[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if b.get(j) != Some(&b'(') {
                            continue;
                        }
                        // find the matching ')'
                        let open = j;
                        let mut d = 0;
                        while j < b.len() {
                            match b[j] {
                                b'(' => d += 1,
                                b')' => {
                                    d -= 1;
                                    if d == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        if j >= b.len() {
                            return None;
                        }
                        let expr = src[open + 1..j].trim().to_string();
                        let mut k = j + 1;
                        while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
                            k += 1;
                        }
                        if b.get(k) == Some(&b';') {
                            k += 1;
                        }
                        let expr = if expr.is_empty() { "0".into() } else { expr };
                        return Some((src[..i].to_string(), expr, src[k..].to_string()));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
