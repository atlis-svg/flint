// No console window for the GUI apps on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use flint::{app, export, flx, hub, model, runtime};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--selftest") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| ".".into());
        return selftest(std::path::Path::new(&out)).map_err(|e| eframe::Error::AppCreation(e.into()));
    }
    if let Some(i) = args.iter().position(|a| a == "--showcase") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "showcase.gif".into());
        return export::export_gif(&showcase(), std::path::Path::new(&out)).map(|n| println!("{n} frames -> {out}")).map_err(|e| eframe::Error::AppCreation(e.into()));
    }
    if args.iter().any(|a| a == "--hub-test") {
        return hub_test().map_err(|e| eframe::Error::AppCreation(e.into()));
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Flint").with_inner_size([1500.0, 920.0]).with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Flint", opts, Box::new(|cc| Ok(Box::new(app::FlintApp::new(cc)))))
}

/// Headless check: builds a small animation with both tween kinds and exports it.
fn selftest(dir: &std::path::Path) -> Result<(), String> {
    use model::*;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut doc = Document::new(320.0, 200.0, 12.0);
    let id = doc.alloc_id();
    let ball = Shape { id, geom: Geom::Ellipse { rx: 20.0, ry: 20.0 }, xf: Xform::at(40.0, 60.0), stroke: Some([0, 0, 0, 255]), stroke_w: 3.0, fill: Some([255, 60, 60, 255]), ..Default::default() };
    let tid = doc.alloc_id();
    let txt = Shape { id: tid, geom: Geom::Text { text: "Flint".into(), size: 32.0 }, xf: Xform::at(10.0, 150.0), stroke: None, stroke_w: 1.0, fill: Some([20, 20, 120, 255]), ..Default::default() };
    let l = &mut doc.layers[0];
    l.keys.get_mut(&0).unwrap().shapes = vec![ball, txt];
    insert_keyframe(l, 11, false);
    for s in &mut l.keys.get_mut(&11).unwrap().shapes {
        if s.id == id {
            s.xf.x = 280.0;
            s.xf.sx = 1.5;
            s.fill = Some([60, 60, 255, 255]);
        }
    }
    l.keys.get_mut(&0).unwrap().tween.kind = TweenKind::Classic;
    // shape tween layer: rect morphs into a star-ish path
    let mut l2 = Layer::new("Morph", 12, LAYER_COLORS[1]);
    let rid = doc.next_id + 1;
    doc.next_id += 3;
    l2.keys.get_mut(&0).unwrap().shapes.push(Shape { id: rid, geom: Geom::Rect { w: 50.0, h: 50.0, radius: 6.0 }, xf: Xform::at(160.0, 140.0), stroke: None, stroke_w: 1.0, fill: Some([40, 180, 90, 255]), ..Default::default() });
    l2.keys.get_mut(&0).unwrap().tween.kind = TweenKind::Shape;
    let star: Vec<P> = (0..10).map(|i| { let a = i as f32 / 10.0 * std::f32::consts::TAU; let r = if i % 2 == 0 { 40.0 } else { 16.0 }; [a.cos() * r, a.sin() * r] }).collect();
    l2.keys.insert(11, Keyframe { shapes: vec![Shape { id: rid + 1, geom: Geom::Path { pts: star, closed: true }, xf: Xform::at(160.0, 140.0), stroke: None, stroke_w: 1.0, fill: Some([240, 200, 40, 255]), ..Default::default() }], ..Default::default() });
    doc.layers.push(l2);
    let mid = doc.layers[0].shapes_at(6);
    let x = mid.iter().find(|s| s.id == id).unwrap().xf.x;
    assert!(x > 40.0 && x < 280.0, "classic tween x = {x}");
    let morph = doc.layers[1].shapes_at(5);
    assert!(matches!(morph[0].geom, Geom::Path { .. }), "shape tween should morph");
    // timeline ops
    let mut t = doc.layers[0].clone();
    insert_frame(&mut t, 3);
    assert_eq!(t.len, 13);
    assert!(t.keys.contains_key(&12));
    remove_frame(&mut t, 3);
    assert_eq!(t.len, 12);
    assert!(t.keys.contains_key(&11));
    let json = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
    let back: Document = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    assert_eq!(back.frame_count(), 12);
    std::fs::write(dir.join("selftest.flint"), json).map_err(|e| e.to_string())?;
    export::export_gif(&doc, &dir.join("selftest.gif"))?;
    export::export_png(&doc, 6, &dir.join("selftest_f7.png"))?;
    export::export_svg(&doc, 6, &dir.join("selftest_f7.svg"))?;
    group_selftest()?;
    timer_selftest()?;
    symbol_selftest(dir)?;
    fill_selftest()?;
    scripted_selftest(dir)?;
    println!("selftest ok: {} frames, outputs in {}", doc.frame_count(), dir.display());
    Ok(())
}

/// A small interactive movie: arrow keys move the player, clicking the button recolors it,
/// a frame script loops on a label, and a score counts up every frame.
pub fn demo_movie() -> model::Document {
    use model::*;
    let mut doc = Document::new(640.0, 400.0, 30.0);
    doc.background = [24, 28, 40, 255];
    let sh = |doc: &mut Document, name: &str, geom: Geom, x: f32, y: f32, fill: Rgba, on_click: &str| Shape {
        id: doc.alloc_id(),
        name: name.into(),
        geom,
        xf: Xform::at(x, y),
        stroke: Some([255, 255, 255, 255]),
        stroke_w: 2.0,
        fill: Some(fill),
        on_click: on_click.into(),
    };
    let hero = sh(&mut doc, "hero", Geom::Ellipse { rx: 22.0, ry: 22.0 }, 320.0, 200.0, [255, 90, 90, 255], "");
    let goal = sh(&mut doc, "goal", Geom::Rect { w: 40.0, h: 40.0, radius: 6.0 }, 560.0, 90.0, [90, 220, 120, 255], "");
    let button = sh(&mut doc, "button", Geom::Rect { w: 150.0, h: 44.0, radius: 10.0 }, 100.0, 360.0, [60, 110, 230, 255],
        "clicks += 1;\nhero.fill = [\"#ff5a5a\", \"#ffd23c\", \"#5ac8ff\", \"#c87aff\"][clicks % 4];\ntrace(`button clicked ${clicks} times`);");
    let mut label = sh(&mut doc, "label", Geom::Text { text: "Recolor".into(), size: 22.0 }, 58.0, 346.0, [255, 255, 255, 255], "");
    label.stroke = None;
    let mut score = sh(&mut doc, "score", Geom::Text { text: "Score: 0".into(), size: 24.0 }, 16.0, 12.0, [255, 255, 255, 255], "");
    score.stroke = None;
    let mut help = sh(&mut doc, "help", Geom::Text { text: "Arrow keys to move, reach the green box".into(), size: 16.0 }, 330.0, 16.0, [180, 190, 210, 255], "");
    help.stroke = None;
    // spinner animated purely on the timeline with a classic tween
    let spinner = sh(&mut doc, "spinner", Geom::Rect { w: 30.0, h: 30.0, radius: 0.0 }, 590.0, 340.0, [255, 200, 60, 255], "");
    let l = &mut doc.layers[0];
    l.name = "Stage".into();
    l.keys.get_mut(&0).unwrap().shapes = vec![goal, hero, button, label, score, help];
    l.len = 30;
    let mut anim = Layer::new("Spinner", 30, LAYER_COLORS[3]);
    anim.keys.get_mut(&0).unwrap().shapes.push(spinner);
    anim.keys.get_mut(&0).unwrap().tween = Tween { kind: TweenKind::Classic, ease: 0.0, spins: 1 };
    insert_keyframe(&mut anim, 29, false);
    let mut actions = Layer::new("Actions", 30, LAYER_COLORS[4]);
    actions.keys.get_mut(&0).unwrap().label = "loop".into();
    insert_keyframe(&mut actions, 29, true);
    actions.keys.get_mut(&29).unwrap().script = "goto_and_play(\"loop\");".into();
    doc.layers.insert(0, actions);
    doc.layers.push(anim);
    doc.script = "let hero = obj(\"hero\");\nlet clicks = 0;\nlet points = 0;\nlet speed = 6;\ntrace(\"movie started\");".into();
    doc.on_frame = "if key_down(\"Left\") { hero.x -= speed; }\nif key_down(\"Right\") { hero.x += speed; }\nif key_down(\"Up\") { hero.y -= speed; }\nif key_down(\"Down\") { hero.y += speed; }\nhero.x = hero.x.max(22.0).min(stage_width() - 22.0);\nhero.y = hero.y.max(22.0).min(stage_height() - 22.0);\nif hero.hit_test(obj(\"goal\")) {\n    points += 1;\n    let sc = obj(\"score\");\n    sc.text = `Score: ${points}`;\n    let g = obj(\"goal\");\n    g.x = random_int(40, 600);\n    g.y = random_int(60, 300);\n}".into();
    doc.on_key = "if key == \"Space\" { if is_playing() { stop(); } else { play(); } }".into();
    doc
}

fn scripted_selftest(dir: &std::path::Path) -> Result<(), String> {
    let doc = demo_movie();
    let errs = runtime::check_scripts(&doc);
    assert!(errs.is_empty(), "script errors: {errs:?}");
    // .flx round trip
    let bytes = flx::encode(&doc)?;
    let back = flx::decode(&bytes)?;
    assert_eq!(back.frame_count(), 30);
    flx::save(&doc, &dir.join("demo.flx"))?;
    std::fs::write(dir.join("demo.flint"), serde_json::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut rt = runtime::Runtime::new(back);
    let step = 1.0 / 30.0;
    // timeline loops via the frame-30 script back to label "loop" (frame 1)
    for _ in 0..40 {
        rt.tick(step);
    }
    assert!(rt.frame() < 30 && rt.playing());
    // keyboard movement through the every-frame script
    let x0 = rt.shapes().iter().find(|(_, s)| s.name == "hero").unwrap().1.xf.x;
    rt.set_input([0.0, 0.0], ["Right".to_string()].into_iter().collect());
    for _ in 0..5 {
        rt.tick(step);
    }
    let x1 = rt.shapes().iter().find(|(_, s)| s.name == "hero").unwrap().1.xf.x;
    assert!((x1 - x0 - 30.0).abs() < 0.01, "hero moved {}", x1 - x0);
    // click the button: recolors the hero
    rt.click([100.0, 360.0]);
    let fill = rt.shapes().iter().find(|(_, s)| s.name == "hero").unwrap().1.fill.unwrap();
    assert_eq!(fill, [0xff, 0xd2, 0x3c, 255]);
    // space toggles playback
    rt.key_pressed("Space");
    assert!(!rt.playing());
    let log = rt.take_log();
    assert!(log.iter().any(|l| l == "movie started") && log.iter().any(|l| l.contains("clicked 1 times")), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("error")), "{log:?}");
    // runtime errors are reported, not fatal
    let mut bad = model::Document::new(100.0, 100.0, 10.0);
    bad.script = "let a = 1; a.nope();".into();
    let mut rt = runtime::Runtime::new(bad);
    rt.tick(0.2);
    assert!(rt.take_log().iter().any(|l| l.contains("error")));
    // projector packaging round trip
    if let Some(player) = flx::player_exe() {
        let out = dir.join(if cfg!(windows) { "demo-projector.exe" } else { "demo-projector" });
        flx::write_projector(&player, &doc, &out)?;
        let exe = std::fs::read(&out).map_err(|e| e.to_string())?;
        let movie = flx::decode(flx::embedded(&exe).ok_or("no embedded movie")?)?;
        flx::embedded_in_file(&out).ok_or("tail read failed")??;
        assert_eq!(movie.frame_count(), 30);
        println!("projector ok: {} ({} KB)", out.display(), exe.len() / 1024);
    }
    println!("scripting ok: demo.flx {} bytes", bytes.len());
    Ok(())
}

fn group_selftest() -> Result<(), String> {
    use model::*;
    let a = Shape { id: 1, geom: Geom::Rect { w: 20.0, h: 20.0, radius: 0.0 }, xf: Xform::at(-30.0, 0.0), fill: Some([255, 0, 0, 255]), ..Default::default() };
    let b = Shape { id: 2, geom: Geom::Ellipse { rx: 10.0, ry: 10.0 }, xf: Xform::at(30.0, 0.0), fill: Some([0, 0, 255, 255]), ..Default::default() };
    let mut g = Shape { id: 3, geom: Geom::Group { children: vec![a, b] }, xf: Xform::at(100.0, 100.0), name: "pair".into(), ..Default::default() };
    let bd = g.bounds();
    assert!((bd[0] - 60.0).abs() < 0.5 && (bd[2] - 140.0).abs() < 0.5, "group bounds {bd:?}");
    assert!(g.hit([70.0, 100.0], 0.0) && g.hit([130.0, 100.0], 0.0) && !g.hit([100.0, 100.0], 0.0));
    g.xf.rot = std::f32::consts::FRAC_PI_2; // children now above/below
    assert!(g.hit([100.0, 70.0], 0.0) && !g.hit([70.0, 100.0], 0.0));
    let leaves = g.flatten();
    assert_eq!(leaves.len(), 2);
    let c = compose(&g.xf, &Xform::at(-30.0, 0.0));
    assert!((c.x - 100.0).abs() < 1e-3 && (c.y - 70.0).abs() < 1e-3);
    // a group scripted by instance name moves as one
    let mut doc = Document::new(200.0, 200.0, 10.0);
    doc.layers[0].keys.get_mut(&0).unwrap().shapes.push(g);
    doc.script = "let p = obj(\"pair\"); p.x = 50;".into();
    let rt = runtime::Runtime::new(doc);
    let moved = rt.shapes()[0].1.bounds();
    assert!(moved[0] < 50.0 && moved[2] > 50.0, "{moved:?}");
    println!("groups ok");
    Ok(())
}

fn timer_selftest() -> Result<(), String> {
    let mut doc = model::Document::new(100.0, 100.0, 10.0);
    doc.script = r#"
let n = 0;
trace("a");
sleep(0.5);
trace("b");
wait(0.5)
trace(`c ${n}`);
"#.into();
    doc.on_key = r#"
if key == "Space" {
    after(0.3, || { trace("later"); });
    let t = every(0.2, || { n += 1; if n >= 3 { stop_timer(); } });
}
if key == "X" { if true { sleep(1); sleep(0.5); } }
"#.into();
    let mut rt = runtime::Runtime::new(doc);
    let tick = |rt: &mut runtime::Runtime, secs: f64| {
        for _ in 0..(secs * 100.0).round() as usize {
            rt.tick(0.01);
        }
    };
    assert_eq!(rt.take_log(), vec!["a"]);
    tick(&mut rt, 0.3);
    assert!(rt.take_log().is_empty());
    rt.key_pressed("Space");
    tick(&mut rt, 0.35);
    let log = rt.take_log();
    assert!(log.contains(&"b".to_string()) && log.contains(&"later".to_string()), "{log:?}");
    tick(&mut rt, 1.5);
    let log = rt.take_log();
    // every() ran 3 times then cancelled itself, so n == 3 when "c" prints
    assert_eq!(log, vec!["c 3"], "{log:?}");
    rt.key_pressed("X");
    tick(&mut rt, 0.05);
    let log = rt.take_log();
    assert!(log.len() == 2 && log.iter().all(|l| l.contains("own line")), "{log:?}");
    println!("timers ok");
    Ok(())
}

/// End-to-end check against a Flint Hub server (FLINT_HUB, default the live one):
/// register, publish the demo, browse, download, like, comment, update, delete.
fn hub_test() -> Result<(), String> {
    let mut h = hub::Hub::load();
    h.session = None;
    let name = format!("test{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() % 1_000_000_000);
    let s = h.register(&name, "hunter22")?;
    assert!(h.register(&name, "hunter22").is_err(), "duplicate username accepted");
    h.session = Some(h.login(&name, "hunter22")?);
    assert!(h.login(&name, "wrong-pass").is_err());
    let _ = s;
    let doc = demo_movie();
    let info = hub::PublishInfo { title: "Hub test movie".into(), description: "made by --hub-test".into(), kind: "game".into(), tags: "test demo".into(), update: None };
    let id = h.publish(&doc, &info, 0)?;
    let (list, _) = h.list("game", "new", "Hub test", "", 0)?;
    assert!(list.iter().any(|m| m.id == id), "published movie not listed");
    let (anims, _) = h.list("animation", "new", "", &name, 0)?;
    assert!(anims.is_empty(), "kind filter broken");
    let got = h.download(id, true)?;
    assert_eq!(got.frame_count(), doc.frame_count());
    let thumb = h.thumb(id)?;
    assert!(tiny_skia::Pixmap::decode_png(&thumb).is_ok(), "thumbnail is not a PNG");
    assert_eq!(h.like(id)?, (true, 1));
    assert_eq!(h.like(id)?, (false, 0));
    h.like(id)?;
    h.comment(id, "nice one")?;
    let cs = h.comments(id)?;
    assert!(cs.first().is_some_and(|c| c.body == "nice one" && c.author == name));
    let m = h.movie(id)?;
    assert!(m.plays == 1 && m.likes == 1 && m.comments == 1, "{m:?}");
    let upd = hub::PublishInfo { title: "Hub test movie v2".into(), update: Some(id), ..info.clone() };
    assert_eq!(h.publish(&doc, &upd, 0)?, id);
    assert_eq!(h.movie(id)?.title, "Hub test movie v2");
    // someone else can't delete it
    let mut other = h.clone();
    other.session = Some(other.register(&format!("{name}b"), "hunter22")?);
    assert!(other.delete(id).is_err());
    h.delete(id)?;
    assert!(h.movie(id).is_err());
    println!("hub ok: {} as {name}", h.base);
    Ok(())
}

/// Platformer-style symbols: a hero with idle/run animations driven by labels, frame scripts
/// inside the symbol, a global function, and an unnamed play-once coin.
pub fn platformer_movie() -> model::Document {
    use model::*;
    let mut doc = Document::new(400.0, 300.0, 10.0);
    let rect = |doc: &mut Document, color: Rgba| Shape { id: doc.alloc_id(), geom: Geom::Rect { w: 40.0, h: 60.0, radius: 4.0 }, fill: Some(color), ..Default::default() };
    // Hero: frames 1-5 "idle" (red), 6-10 "run" (blue); the last frame of each loops back to its label
    let hero_id = doc.alloc_id();
    let mut l = Layer::new("Body", 10, LAYER_COLORS[0]);
    let red = rect(&mut doc, [220, 40, 40, 255]);
    let blue = rect(&mut doc, [40, 80, 220, 255]);
    l.keys.get_mut(&0).unwrap().shapes = vec![red];
    l.keys.get_mut(&0).unwrap().label = "idle".into();
    l.keys.insert(5, Keyframe { shapes: vec![blue], label: "run".into(), ..Default::default() });
    let mut acts = Layer::new("Actions", 10, LAYER_COLORS[4]);
    insert_keyframe(&mut acts, 4, true);
    acts.keys.get_mut(&4).unwrap().script = "me.goto_and_play(\"idle\");".into();
    insert_keyframe(&mut acts, 9, true);
    acts.keys.get_mut(&9).unwrap().script = "me.goto_and_play(\"run\");\nshout(\"run loop\");".into();
    doc.symbols.push(Symbol { id: hero_id, name: "Hero".into(), layers: vec![acts, l] });
    // Coin: 4 frames, plays once and stops itself on the last frame
    let coin_id = doc.alloc_id();
    let mut cl = Layer::new("Coin", 4, LAYER_COLORS[3]);
    let gold = Shape { id: doc.alloc_id(), geom: Geom::Ellipse { rx: 8.0, ry: 8.0 }, fill: Some([250, 200, 40, 255]), ..Default::default() };
    cl.keys.get_mut(&0).unwrap().shapes = vec![gold];
    insert_keyframe(&mut cl, 3, false);
    cl.keys.get_mut(&3).unwrap().script = "me.stop(); shout(\"coin done\");".into();
    doc.symbols.push(Symbol { id: coin_id, name: "Coin".into(), layers: vec![cl] });
    // stage
    let hero = Shape { id: doc.alloc_id(), name: "hero".into(), geom: Geom::Symbol { symbol: hero_id, first: 0, mode: SymMode::Loop, children: vec![] }, xf: Xform::at(100.0, 200.0), ..Default::default() };
    let coin = Shape { id: doc.alloc_id(), geom: Geom::Symbol { symbol: coin_id, first: 0, mode: SymMode::Loop, children: vec![] }, xf: Xform::at(300.0, 100.0), ..Default::default() };
    doc.layers[0].keys.get_mut(&0).unwrap().shapes = vec![hero, coin];
    doc.functions = "fn run(o) { o.goto_and_play(\"run\"); }\nfn shout(msg) { trace(`>> ${msg}`); }".into();
    doc.script = "let hero = obj(\"hero\");".into();
    doc.on_key = "if key == \"R\" { run(hero); }".into();
    doc
}

fn symbol_selftest(dir: &std::path::Path) -> Result<(), String> {
    use model::*;
    let doc = platformer_movie();
    assert!(runtime::check_scripts(&doc).is_empty(), "{:?}", runtime::check_scripts(&doc));
    // editor view: instances expand, are hittable and animate with the parent timeline
    let list = doc.render_list(0);
    let hero = &list.iter().find(|(_, s)| s.name == "hero").unwrap().1;
    assert_eq!(hero.flatten().len(), 1);
    assert!(hero.hit([100.0, 200.0], 0.0) && !hero.hit([200.0, 200.0], 0.0));
    assert!(doc.layer_shapes(0, 0).iter().any(|s| s.hit([300.0, 100.0], 0.0)), "coin not hittable");
    // raster export sees the instance
    let pm = export::render_frame(&doc, 0, 1.0);
    let px = pm.pixel(100, 200).unwrap().demultiply();
    assert!(px.red() > 200 && px.blue() < 80, "hero not rendered red");
    // symbol editing swaps timelines and unwinds for saving
    let mut ed = doc.clone();
    let hero_sym = ed.symbols[0].id;
    assert!(ed.enter_symbol(EditCtx { symbol: hero_sym, frame: 0, layer: 0, pan: [0.0; 2], zoom: 1.0 }));
    assert_eq!(ed.frame_count(), 10);
    ed.layers[1].keys.get_mut(&0).unwrap().label = "stand".into();
    assert!(ed.symbol_in_use(hero_sym));
    let saved: Document = serde_json::from_str(&serde_json::to_string(&ed.normalized()).unwrap()).unwrap();
    assert_eq!(saved.layers.len(), 1);
    assert_eq!(saved.symbols[0].label_frame("stand"), Some(0));
    ed.exit_symbol();
    assert_eq!(ed.frame_count(), 1);

    // player: hero loops idle by itself, runs on R through a global function, coin stops itself
    let mut rt = runtime::Runtime::new(doc.clone());
    let hero_color = |rt: &runtime::Runtime| rt.shapes().into_iter().find(|(_, s)| s.name == "hero").unwrap().1.flatten()[0].fill.unwrap();
    for _ in 0..30 {
        rt.tick(0.1);
    }
    assert_eq!(hero_color(&rt)[0], 220, "hero should still be idle (red)");
    let log = rt.take_log();
    assert_eq!(log.iter().filter(|l| l.contains("coin done")).count(), 1, "{log:?}");
    assert!(!log.iter().any(|l| l.contains("error")), "{log:?}");
    rt.key_pressed("R");
    for _ in 0..3 {
        rt.tick(0.1);
    }
    assert_eq!(hero_color(&rt)[2], 220, "hero should be running (blue)");
    for _ in 0..20 {
        rt.tick(0.1);
    }
    assert_eq!(hero_color(&rt)[2], 220, "run should loop on itself");
    let log = rt.take_log();
    assert!(log.iter().any(|l| l == ">> run loop"), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("error")), "{log:?}");
    flx::save(&doc, &dir.join("platformer.flx"))?;
    std::fs::write(dir.join("platformer.flint"), serde_json::to_string(&doc).unwrap()).map_err(|e| e.to_string())?;
    println!("symbols ok");
    Ok(())
}

/// The README preview: classic tweens with easing and spins, a shape morph, and text, rendered by Flint itself.
fn showcase() -> model::Document {
    use model::*;
    let mut doc = Document::new(640.0, 300.0, 30.0);
    doc.background = [24, 26, 38, 255];
    let n = 48;
    let mut id = || { doc.next_id += 1; doc.next_id };
    let mk = |id: u64, geom: Geom, x: f32, y: f32, fill: Rgba, stroke: Option<Rgba>| Shape { id, geom, xf: Xform::at(x, y), fill: Some(fill), stroke, stroke_w: 4.0, ..Default::default() };
    let mut layers = Vec::new();
    // bouncing ball: down with ease-in, up with ease-out, squash on landing
    let bid = id();
    let mut l = Layer::new("Ball", n, LAYER_COLORS[0]);
    let ball = |y: f32, sx: f32, sy: f32| { let mut b = mk(bid, Geom::Ellipse { rx: 28.0, ry: 28.0 }, 110.0, y, [255, 92, 92, 255], Some([255, 255, 255, 255])); b.xf.sx = sx; b.xf.sy = sy; b };
    l.keys.get_mut(&0).unwrap().shapes = vec![ball(60.0, 1.0, 1.0)];
    l.keys.get_mut(&0).unwrap().tween = Tween { kind: TweenKind::Classic, ease: -100.0, spins: 0 };
    l.keys.insert(22, Keyframe { shapes: vec![ball(232.0, 1.35, 0.7)], tween: Tween { kind: TweenKind::Classic, ease: 100.0, spins: 0 }, ..Default::default() });
    l.keys.insert(26, Keyframe { shapes: vec![ball(225.0, 0.9, 1.1)], tween: Tween { kind: TweenKind::Classic, ease: 100.0, spins: 0 }, ..Default::default() });
    l.keys.insert(n - 1, Keyframe { shapes: vec![ball(60.0, 1.0, 1.0)], ..Default::default() });
    layers.push(l);
    // morph: rounded square -> star -> square
    let sq = |id: u64| mk(id, Geom::Rect { w: 110.0, h: 110.0, radius: 18.0 }, 320.0, 150.0, [80, 200, 140, 255], None);
    let star: Vec<P> = (0..10).map(|i| { let a = i as f32 / 10.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2; let r = if i % 2 == 0 { 78.0 } else { 34.0 }; [a.cos() * r, a.sin() * r] }).collect();
    let mut l = Layer::new("Morph", n, LAYER_COLORS[2]);
    let (a, b, c) = (id(), id(), id());
    l.keys.get_mut(&0).unwrap().shapes = vec![sq(a)];
    l.keys.get_mut(&0).unwrap().tween = Tween { kind: TweenKind::Shape, ease: 50.0, spins: 0 };
    l.keys.insert(24, Keyframe { shapes: vec![mk(b, Geom::Path { pts: star, closed: true }, 320.0, 150.0, [255, 200, 60, 255], None)], tween: Tween { kind: TweenKind::Shape, ease: 50.0, spins: 0 }, ..Default::default() });
    l.keys.insert(n - 1, Keyframe { shapes: vec![sq(c)], ..Default::default() });
    layers.push(l);
    // spinning, scaling triangle with a full turn
    let tid = id();
    let tri = |s: f32, fill: Rgba| { let mut t = mk(tid, Geom::Path { pts: vec![[0.0, -55.0], [48.0, 30.0], [-48.0, 30.0]], closed: true }, 530.0, 150.0, fill, Some([255, 255, 255, 255])); t.xf.sx = s; t.xf.sy = s; t };
    let mut l = Layer::new("Spin", n, LAYER_COLORS[4]);
    l.keys.get_mut(&0).unwrap().shapes = vec![tri(0.8, [120, 140, 255, 255])];
    l.keys.get_mut(&0).unwrap().tween = Tween { kind: TweenKind::Classic, ease: 0.0, spins: 1 };
    l.keys.insert(n - 1, Keyframe { shapes: vec![tri(0.8, [120, 140, 255, 255])], ..Default::default() });
    l.keys.insert(24, Keyframe { shapes: vec![tri(1.25, [200, 110, 255, 255])], tween: Tween { kind: TweenKind::Classic, ease: 0.0, spins: 0 }, ..Default::default() });
    layers.push(l);
    // title
    let mut l = Layer::new("Title", n, LAYER_COLORS[1]);
    let mut t = mk(id(), Geom::Text { text: "Flint".into(), size: 30.0 }, 20.0, 252.0, [235, 235, 245, 255], None);
    t.stroke = None;
    let mut sub = mk(id(), Geom::Text { text: "tweens · shape morphs · made & exported by Flint".into(), size: 15.0 }, 100.0, 264.0, [150, 155, 180, 255], None);
    sub.stroke = None;
    l.keys.get_mut(&0).unwrap().shapes = vec![t, sub];
    layers.push(l);
    layers.reverse();
    doc.layers = layers;
    doc
}

fn fill_selftest() -> Result<(), String> {
    use flint::fill::{FillError, region_fill};
    use model::*;
    let line = |a: P, b: P| Shape { geom: Geom::Path { pts: vec![a, b], closed: false }, stroke: Some([0, 0, 0, 255]), stroke_w: 2.0, ..Default::default() };
    let area = |p: &[P]| (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f32>().abs() * 0.5;
    // a box drawn with four separate lines (the case the old bucket couldn't fill)
    let bx = vec![line([0.0, 0.0], [100.0, 0.0]), line([100.0, 0.0], [100.0, 100.0]), line([100.0, 100.0], [0.0, 100.0]), line([0.0, 100.0], [0.0, 0.0])];
    let poly = region_fill(&bx, [50.0, 50.0], 1.0, 0.0).map_err(|e| format!("box: {e:?}"))?;
    assert!(point_in_poly([50.0, 50.0], &poly) && point_in_poly([5.0, 95.0], &poly) && !point_in_poly([120.0, 50.0], &poly));
    let a = area(&poly);
    assert!((9000.0..10500.0).contains(&a), "box fill area {a}");
    assert_eq!(region_fill(&bx, [100.0, 50.0], 1.0, 0.0), Err(FillError::OnShape));
    assert_eq!(region_fill(&bx, [150.0, 50.0], 1.0, 0.0), Err(FillError::NotEnclosed));
    // a 6px opening: leaks with no gap closing, fills with Small/Medium
    let mut open = bx.clone();
    open[0] = line([0.0, 0.0], [94.0, 0.0]);
    assert_eq!(region_fill(&open, [50.0, 50.0], 1.0, 0.0), Err(FillError::NotEnclosed));
    assert!(region_fill(&open, [50.0, 50.0], 1.0, 3.0).is_ok());
    // donut: box with a circle outline inside; filling between them leaves the circle as a hole
    let mut donut = bx.clone();
    donut.push(Shape { geom: Geom::Ellipse { rx: 20.0, ry: 20.0 }, xf: Xform::at(50.0, 50.0), stroke: Some([0, 0, 0, 255]), stroke_w: 2.0, ..Default::default() });
    let ring = region_fill(&donut, [10.0, 10.0], 2.0, 0.0).map_err(|e| format!("donut: {e:?}"))?;
    assert!(point_in_poly([10.0, 10.0], &ring) && point_in_poly([90.0, 50.0], &ring) && !point_in_poly([50.0, 50.0], &ring), "hole missing");
    // box interior minus the circle (strokes take ~1px on each side), so about 99² − π·21²
    let a = area(&ring);
    assert!((8000.0..9100.0).contains(&a), "donut area {a}");
    // the editor's triangulator must cover the slit polygon exactly
    let sp: Vec<egui::Pos2> = ring.iter().map(|p| egui::pos2(p[0], p[1])).collect();
    let tri_area: f32 = flint::render::triangulate(&sp).iter().map(|t| {
        let (a, b, c) = (sp[t[0] as usize], sp[t[1] as usize], sp[t[2] as usize]);
        ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
    }).sum();
    assert!((tri_area - a).abs() / a < 0.02, "triangulated {tri_area} vs polygon {a}");
    // and the exported raster shows the hole
    let mut doc = Document::new(100.0, 100.0, 10.0);
    let fill = Shape { geom: Geom::Path { pts: ring.clone(), closed: true }, fill: Some([0, 200, 0, 255]), ..Default::default() };
    doc.layers[0].keys.get_mut(&0).unwrap().shapes = vec![fill];
    let pm = export::render_frame(&doc, 0, 1.0);
    let g = |x, y| pm.pixel(x, y).unwrap().green();
    assert!(g(10, 10) == 200 && g(50, 50) == 255 && pm.pixel(50, 50).unwrap().red() == 255, "raster hole");
    println!("fill ok");
    Ok(())
}
