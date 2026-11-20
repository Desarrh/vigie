#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(deprecated)]

mod app;
mod cli;
mod docker;
mod hunter;
mod journal;
mod pdh;
mod qol;
mod sampler;
mod startup;
mod sys;
mod travail;
mod ui;
mod winfo;

/// Tests qui lancent de vrais processus jetables : un à la fois (sinon la sieste de projet
/// endormirait les processus des autres tests, tous sous C:\Dev\vigie).
#[cfg(test)]
pub static TEST_PROCESSUS: std::sync::Mutex<()> = std::sync::Mutex::new(());

use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Rendu sans GPU, comme l'Explorateur : egui dessine sur le processeur, softbuffer copie l'image
/// dans la fenêtre (GDI). Pas de contexte OpenGL/DirectX ⇒ pas des ~150 Mo du pilote graphique,
/// et Vigie ne réveille jamais le GPU pendant qu'un jeu tourne.
struct Gfx {
    window: Rc<Window>,
    egui: egui_winit::State,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    renderer: EguiSoftwareRender,
}

enum UserEvent {
    RepaintAt(Instant),
    /// une autre instance a été lancée (Ctrl+Maj+Échap, raccourci…) : ramener la fenêtre
    Front,
    /// nouvelle mesure arrivée
    Sample,
}

struct Runner {
    ctx: egui::Context,
    gfx: Option<Gfx>,
    app: Option<app::App>,
    settings: Option<app::Settings>,
    next_repaint: Option<Instant>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
}

impl Runner {
    fn paint(&mut self) {
        let (Some(gfx), Some(app)) = (self.gfx.as_mut(), self.app.as_mut()) else { return };
        let size = gfx.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let t0 = Instant::now();
        let mut raw = gfx.egui.take_egui_input(&gfx.window);
        socle::testmode::prepare_input(&mut raw); // sans effet hors mode test
        let out = self.ctx.run_ui(raw, |ui| {
            app.tick();
            app.handle_keys(ui.ctx());
            app.draw(ui);
            app.apply_actions();
        });
        gfx.egui.handle_platform_output(&gfx.window, out.platform_output);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        if gfx.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = gfx.surface.buffer_mut() else { return };
        let bg = app.pal.bg;
        buf.fill(((bg.r() as u32) << 16) | ((bg.g() as u32) << 8) | bg.b() as u32);
        {
            let px: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buf[..]);
            let mut target = BufferMutRef::new(px, w.get() as usize, h.get() as usize);
            gfx.renderer.render(&mut target, &prims, &out.textures_delta, out.pixels_per_point);
        }
        let _ = buf.present();
        app.last_frame_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn save(&mut self) {
        if let Some(app) = self.app.as_mut() {
            if let Some(g) = &self.gfx {
                let sz = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
                if sz.width > 100.0 {
                    app.s.window = Some((sz.width, sz.height));
                }
            }
            app.s.save();
        }
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let (w, h) = self.settings.as_ref().and_then(|s| s.window).unwrap_or((1240.0, 780.0));
        let attrs = Window::default_attributes()
            .with_title("Vigie")
            .with_inner_size(LogicalSize::new(w.max(820.0), h.max(500.0)))
            .with_min_inner_size(LogicalSize::new(820.0, 500.0))
            .with_window_icon(window_icon());
        // Règle n°0 : en test (--test-hors-ecran), fenêtre hors écran, sans focus, hors barre des tâches.
        let test = socle::testmode::enabled();
        // socle v0.1.2 : invisible et sans focus à la création, puis `hide` (opacité 0, au fond de la pile, à l'écran pour rester dessinée)
        let attrs = if test { attrs.with_visible(false).with_active(false) } else { attrs };
        let window = Rc::new(el.create_window(attrs).expect("création de la fenêtre"));
        if test {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = window.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    socle::testmode::hide(w.hwnd.get());
                    // mesure du mode arrière-plan : réduite sans activer (aucun changement de focus)
                    if std::env::var_os("VIGIE_TEST_REDUIT").is_some() {
                        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(w.hwnd.get() as _, windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE) };
                    }
                }
            }
        }
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        let egui = egui_winit::State::new(self.ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), None, Some(8192));
        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
        if self.app.is_none() {
            let s = self.settings.take().unwrap_or_default();
            let mut a = app::App::new(&self.ctx, s);
            let px = self.proxy.clone();
            let _ = a.waker.set(Box::new(move || {
                let _ = px.send_event(UserEvent::Sample);
            }));
            if let Some(msg) = socle::crash::pending("Vigie") {
                a.toast(msg, true);
            }
            self.app = Some(a);
        }
        window.request_redraw();
        self.gfx = Some(Gfx { window, egui, _context: context, surface, renderer });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let resp = gfx.egui.on_window_event(&gfx.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                self.save();
                el.exit();
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Occluded(o) => {
                if let Some(app) = self.app.as_mut() {
                    app.set_background(o);
                }
                if !o {
                    gfx.window.request_redraw();
                }
            }
            WindowEvent::Resized(sz) => {
                let min = sz.width == 0 || sz.height == 0 || gfx.window.is_minimized().unwrap_or(false);
                if let Some(app) = self.app.as_mut() {
                    app.set_background(min);
                }
                gfx.window.request_redraw();
            }
            _ => {
                if resp.repaint {
                    gfx.window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        let t = match ev {
            UserEvent::RepaintAt(t) => t,
            UserEvent::Sample => {
                // réduite : on traite la mesure sans dessiner ; sinon une image normale
                // réduite, ou jeu / vidéo en plein écran par-dessus : mesure traitée, aucune image
                if let Some(app) = self.app.as_mut() {
                    app.tick();
                    if app.minimized || app.fullscreen {
                        return;
                    }
                }
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
                return;
            }
            UserEvent::Front => {
                if let Some(g) = &self.gfx {
                    g.window.set_minimized(false);
                    g.window.focus_window();
                    g.window.request_redraw();
                }
                return;
            }
        };
        if t <= Instant::now() {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        } else {
            self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.next_repaint {
            Some(t) if t <= Instant::now() => {
                self.next_repaint = None;
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }
}

fn window_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// `--avant-jeu` (sans fenêtre, pour le launcher) : chasse les fantômes sûrs, vide la veille (si admin),
/// passe en Performances élevées. Résumé sur la sortie standard et dans %APPDATA%\Vigie\dernier-jeu.txt.
fn headless_game(enter: bool, json: bool) -> i32 {
    let (msg, libere, reveilles) = if enter {
        let ctx = egui::Context::default();
        let saved = app::Settings::load();
        let mut s = saved.clone();
        s.interval_ms = 1500;
        s.gpu = false;
        let mut a = app::App::new(&ctx, s);
        // réglages propres à ce mode : ne pas les enregistrer
        a.s.interval_ms = saved.interval_ms;
        a.s.gpu = saved.gpu;
        a.headless = true;
        let t0 = Instant::now();
        while a.hist.cpu.0.len() < 2 && t0.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(50));
            a.tick();
        }
        a.run_game_mode();
        // sans fenêtre : on attend la fin du travail (thread) avant de répondre
        a.wait_jobs(Duration::from_secs(60));
        (a.last_toast().unwrap_or_default(), a.game_freed, 0)
    } else {
        // léger : ni egui ni thread de mesure
        // reprend un ancien prev_power resté dans settings.json, puis termine le mode jeu
        travail::EtatJeu::migrer(app::Settings::load().prev_power);
        let (msg, _, n) = travail::finir();
        (msg, 0, n)
    };
    if json {
        println!("{}", serde_json::to_string(&cli::Jeu { message: msg.clone(), libere, reveilles }).unwrap_or_default());
    } else {
        println!("{msg}");
    }
    if let Some(d) = app::Settings::dir() {
        let _ = std::fs::write(d.join("dernier-jeu.txt"), &msg);
    }
    0
}

/// Debug : `VIGIE_CAPTURE=x.png` rend l'interface **entièrement hors écran** (aucune fenêtre, aucun focus,
/// rien à l'écran de l'utilisateur) après quelques mesures, l'enregistre puis quitte. Priorité basse.
/// Options : VIGIE_VIEW, VIGIE_SELECT, VIGIE_PALETTE, VIGIE_GAME, VIGIE_DUMP, VIGIE_SIZE=1240x780.
fn capture_offscreen(path: &str) -> i32 {
    unsafe {
        use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
    let (w, h) = std::env::var("VIGIE_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))))
        .unwrap_or((1240, 780));
    let ctx = egui::Context::default();
    let mut app = app::App::new(&ctx, app::Settings::load());
    let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
    let mut px: Vec<[u8; 4]> = vec![[0; 4]; w * h];
    let t0 = Instant::now();
    loop {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
            time: Some(t0.elapsed().as_secs_f64()),
            focused: false,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            app.tick();
            app.draw(ui);
            app.apply_actions();
        });
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let bg = app.pal.bg;
        px.fill([bg.b(), bg.g(), bg.r(), 255]);
        {
            let mut target = BufferMutRef::new(&mut px, w, h);
            renderer.render(&mut target, &prims, &out.textures_delta, out.pixels_per_point);
        }
        let ready = app.hist.cpu.0.len() >= 4 && !app.rows_dirty;
        if ready || t0.elapsed() > Duration::from_secs(20) {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let rgba: Vec<u8> = px.iter().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    let _ = image::save_buffer(path, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
    if let Some(s) = &app.snap {
        let mut dump = format!("{:.1} ms : {}
", s.cost_ms, s.parts);
        if let Ok(name) = std::env::var("VIGIE_DUMP") {
            for p in s.procs.iter().filter(|p| p.name.eq_ignore_ascii_case(&name)) {
                dump += &format!(
                    "{} pid {} parent {:?} session {} window {:?} ports {:?} idle {} age {} cmd {:?}
",
                    p.name, p.key.pid, p.parent, p.session, p.window, p.ports, p.idle_secs, p.age_secs, p.cmd
                );
            }
            for g in &app.ghosts {
                dump += &format!("ghost {} {:?} {}
", g.name, g.kind, g.reason);
            }
        }
        let _ = std::fs::write(format!("{path}.txt"), dump);
    }
    0
}

fn main() {
    socle::crash::install("Vigie", env!("CARGO_PKG_VERSION"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--set-taskmgr") {
        let on = args.get(i + 1).map(|v| v == "on").unwrap_or(false);
        std::process::exit(if sys::set_taskmgr_direct(on).is_ok() { 0 } else { 1 });
    }
    let json = args.iter().any(|a| a == "--json");
    if args.iter().any(|a| a == "--avant-jeu") {
        std::process::exit(headless_game(true, json));
    }
    if args.iter().any(|a| a == "--apres-jeu") {
        std::process::exit(headless_game(false, json));
    }
    // Pour Ultimate (garde-fou dev avant de jouer) : toujours en JSON
    if args.iter().any(|a| a == "--etat-dev") {
        println!("{}", serde_json::to_string(&cli::etat()).unwrap_or_default());
        std::process::exit(0);
    }
    // V3, pour Ultimate : pic de RAM connu d'un jeu et ce qu'il manque maintenant (JSON, ~1 ms)
    if let Some(i) = args.iter().position(|a| a == "--memoire-jeu") {
        let jeu = args.get(i + 1).cloned().unwrap_or_default();
        let s = app::Settings::load();
        let m = sys::memory();
        let avail = m.avail;
        // nom ou chemin d'exe, ou dossier d'installation (plus gros pic connu sous ce dossier)
        let v = match qol::find_game(&s.game_peaks, &jeu) {
            Some((exe, g)) => serde_json::json!({"v": 1, "connu": true, "nom": g.name, "exe": exe, "chemin": g.path, "pic_mo": g.peak >> 20, "disponible_mo": avail >> 20, "manque_mo": qol::missing(g.peak, avail) >> 20}),
            None => serde_json::json!({"v": 1, "connu": false, "disponible_mo": avail >> 20}),
        };
        println!("{v}");
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--couper-dev") {
        let (srv, dk, wsl) = (args.iter().any(|a| a == "--serveurs"), args.iter().any(|a| a == "--docker"), args.iter().any(|a| a == "--wsl"));
        let all = !(srv || dk || wsl);
        let c = cli::couper(all || srv, all || dk, all || wsl);
        if json {
            println!("{}", serde_json::to_string(&c).unwrap_or_default());
        } else {
            println!("{}", c.resume());
        }
        std::process::exit(if c.erreurs.is_empty() { 0 } else { 2 });
    }
    // relancée en admin : l'instance non élevée est en train de se fermer
    let after_elevate = args.iter().any(|a| a == "--after-elevate");
    if let Ok(path) = std::env::var("VIGIE_CAPTURE") {
        std::process::exit(capture_offscreen(&path));
    }
    // Instance unique (socle : mutex + tube nommé) : une 2e instance ramène la fenêtre.
    // Pas en test (règle n°0) ni juste après une élévation (l'ancienne instance se ferme).
    let mut instance = None;
    if socle::testmode::enabled() {
        socle::testmode::low_priority();
    } else if !after_elevate {
        match socle::instance::start("Vigie", &args) {
            socle::instance::Start::Forwarded => return,
            socle::instance::Start::Primary(i) => instance = Some(i),
            socle::instance::Start::Alone => {}
        }
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("boucle d’événements");
    let proxy = event_loop.create_proxy();
    let proxy_app = proxy.clone();
    if let Some(i) = instance {
        let p = proxy.clone();
        i.listen(move |_args| {
            let _ = p.send_event(UserEvent::Front);
        });
    }
    let ctx = egui::Context::default();
    ctx.set_request_repaint_callback(move |info| {
        let _ = proxy.send_event(UserEvent::RepaintAt(Instant::now() + info.delay));
    });
    let mut runner = Runner { ctx, gfx: None, app: None, settings: Some(app::Settings::load()), next_repaint: None, proxy: proxy_app };
    let _ = event_loop.run_app(&mut runner);
}
