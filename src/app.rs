//! État de l'application : réglages, mesures reçues, lignes affichées, actions.
use crate::hunter::{self, Fix, Ghost};
use crate::sampler::{self, Cmd, Key, Proc, Snapshot};
use crate::sys::{self, Prio};
use socle::theme::{self, Palette};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

pub const HIST: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum View {
    #[default]
    Procs,
    Hunter,
    Perf,
    Startup,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Col {
    Name,
    Pid,
    Status,
    Cpu,
    Mem,
    Disk,
    Gpu,
    GpuMem,
    Ports,
    Threads,
}

impl Col {
    pub fn label(self) -> &'static str {
        match self {
            Col::Name => "Nom",
            Col::Pid => "PID",
            Col::Status => "Statut",
            Col::Cpu => "Processeur",
            Col::Mem => "Mémoire",
            Col::Disk => "Disque",
            Col::Gpu => "GPU",
            Col::GpuMem => "Mém. GPU",
            Col::Ports => "Ports",
            Col::Threads => "Threads",
        }
    }
    pub fn width(self) -> f32 {
        match self {
            Col::Name => 0.0,
            Col::Pid => 64.0,
            Col::Status => 124.0,
            Col::Cpu => 86.0,
            Col::Mem => 92.0,
            Col::Disk => 92.0,
            Col::Gpu => 64.0,
            Col::GpuMem => 86.0,
            Col::Ports => 110.0,
            Col::Threads => 70.0,
        }
    }
    pub const OPTIONAL: [Col; 9] = [Col::Pid, Col::Status, Col::Cpu, Col::Mem, Col::Disk, Col::Gpu, Col::GpuMem, Col::Ports, Col::Threads];
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub view: View,
    pub tree: bool,
    pub sort: Col,
    pub asc: bool,
    pub interval_ms: u64,
    pub gpu: bool,
    pub cols: Vec<Col>,
    pub details: bool,
    pub hide_system: bool,
    pub confirm_kill: bool,
    pub whitelist: Vec<String>,
    pub window: Option<(f32, f32)>,
    pub side_w: f32,
    pub game_kill: bool,
    pub game_purge: bool,
    /// mettre en veille (vider la RAM sans fermer) les apps en arrière-plan
    pub game_trim: bool,
    pub game_power: bool,
    /// ancien emplacement du plan à restaurer : lu une fois pour migrer vers mode-jeu.json, jamais réécrit
    #[serde(skip_serializing)]
    pub prev_power: Option<String>,
    /// historique de la recherche (autocomplétion)
    pub search_history: socle::autocomplete::History,
    /// V3 : pic de RAM de chaque jeu (clé = nom de l'exe)
    pub game_peaks: HashMap<String, crate::qol::GamePeak>,
    /// V2 : dossier des projets de dev
    pub dev_root: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            view: View::Procs,
            tree: true,
            sort: Col::Mem,
            asc: false,
            interval_ms: 1000,
            gpu: true,
            cols: vec![Col::Status, Col::Cpu, Col::Mem, Col::Disk, Col::Gpu, Col::Ports],
            details: true,
            hide_system: false,
            confirm_kill: true,
            whitelist: Vec::new(),
            window: None,
            side_w: 210.0,
            game_kill: true,
            game_purge: true,
            game_trim: true,
            game_power: true,
            prev_power: None,
            search_history: Default::default(),
            game_peaks: HashMap::new(),
            dev_root: r"C:\Dev".into(),
        }
    }
}

impl Settings {
    /// `%APPDATA%\Vigie`, ou `VIGIE_DATA_DIR` (tests : ne jamais toucher aux réglages ni au journal de l'utilisateur).
    pub fn dir() -> Option<std::path::PathBuf> {
        if let Some(d) = std::env::var_os("VIGIE_DATA_DIR") {
            return Some(std::path::PathBuf::from(d));
        }
        crate::sys::appdata().map(|d| d.join("Vigie"))
    }
    fn path() -> Option<std::path::PathBuf> {
        Self::dir().map(|d| d.join("settings.json"))
    }
    pub fn load() -> Settings {
        Self::path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            if let Ok(b) = serde_json::to_vec_pretty(self) {
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, b).is_ok() {
                    let _ = std::fs::rename(tmp, p);
                }
            }
        }
    }
}

/// Historique circulaire (une valeur par mesure).
#[derive(Clone, Default)]
pub struct Hist(pub VecDeque<f32>);

impl Hist {
    pub fn push(&mut self, v: f32) {
        if self.0.len() >= HIST {
            self.0.pop_front();
        }
        self.0.push_back(v);
    }
    pub fn max(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }
}

#[derive(Default)]
pub struct Hists {
    pub cpu: Hist,
    pub mem: Hist,
    pub gpu: Hist,
    pub disk: Hist,
    pub net: Hist,
}

#[derive(Clone, Copy, Default)]
pub struct Agg {
    pub cpu: f32,
    pub mem: u64,
    pub disk: f64,
    pub gpu: f32,
    pub gpu_mem: u64,
    pub count: usize,
}

#[derive(Clone)]
pub struct Row {
    pub idx: usize,
    pub depth: u16,
    pub has_kids: bool,
    pub open: bool,
    /// totaux de l'arbre quand il est replié
    pub agg: Option<Agg>,
}

pub enum Confirm {
    Kill { keys: Vec<Key>, tree: bool, names: String },
}


pub enum Action {
    Kill(Vec<Key>, bool),
    KillNow(Vec<Key>, bool),
    Close(Vec<Key>),
    Suspend(Vec<Key>, bool),
    Priority(Vec<Key>, Prio),
    Focus(Key),
    OpenLocation(Key),
    Copy(String),
    Select(Key),
    Clean(Vec<Key>),
    Whitelist(String),
    Unwhitelist(String),
    GameMode,
    GameExit,
    Elevate,
    TaskmgrToggle,
    /// recopie l'exe en cours dans Program Files (UAC)
    TaskmgrUpdate,
    SampleNow,
    SetView(View),
    WebSearch(String),
    Trim(Vec<Key>),
    /// V1 : terminer ce qui écoute sur ce port
    KillPort(u16),
    /// V2 : endormir / réveiller un projet (dossier)
    SleepProject(String),
    WakeProject(String),
    StartupToggle(usize, bool),
    StartupRefresh,
}

/// Textes d'un processus en minuscules, pour la recherche et le tri par nom.
pub struct Low {
    pub name: String,
    pub title: String,
    pub cmd: String,
    pub desc: String,
}

impl Low {
    fn of(p: &Proc, icons: &HashMap<Arc<str>, IconEntry>) -> Low {
        Low {
            name: p.name.to_lowercase(),
            title: p.window.as_deref().unwrap_or("").to_lowercase(),
            cmd: p.cmd.as_deref().unwrap_or("").to_lowercase(),
            desc: p.path.as_deref().and_then(|pa| icons.get(pa)).and_then(|e| e.desc.as_deref()).unwrap_or("").to_lowercase(),
        }
    }
}

pub struct IconEntry {
    pub tex: Option<egui::TextureHandle>,
    pub desc: Option<String>,
}

/// Réveil de la boucle d'événements à chaque mesure (posé par `main`).
pub type Waker = Arc<std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>>;

pub struct App {
    pub waker: Waker,
    pub ctx: egui::Context,
    pub s: Settings,
    pub pal: Palette,
    pub snap: Option<Arc<Snapshot>>,
    rx: Receiver<Arc<Snapshot>>,
    cmd: Sender<Cmd>,
    pub hist: Hists,
    pub phist: HashMap<Key, (Hist, Hist)>,
    pub rows: Vec<Row>,
    pub rows_dirty: bool,
    /// ordre affiché au moment où la souris est entrée dans la liste (pas de lignes qui sautent)
    pub rank: HashMap<Key, usize>,
    pub freeze: bool,
    pub by_key: HashMap<Key, usize>,
    pub selected: HashSet<Key>,
    pub cursor: Option<Key>,
    pub anchor: Option<Key>,
    pub scroll_to_cursor: bool,
    pub filter: String,
    pub focus_search: bool,
    pub collapsed: HashSet<Key>,
    pub ghosts: Vec<Ghost>,
    pub checked: HashMap<Key, bool>,
    pub toasts: socle::ui::Toasts,
    pub confirm: Option<Confirm>,
    pub palette: socle::ui::Palette,
    /// recherche : autocomplétion floue (historique + noms, descriptions, ports en cours)
    pub search_ac: socle::autocomplete::Autocomplete,
    pub search_field: socle::ui::AutoField,
    /// empreinte (processus + icônes connues) des derniers candidats d'autocomplétion
    cand_fp: u64,
    /// minuscules de chaque processus (index = celui de `snap.procs`), calculées une fois par
    /// mesure et seulement si une recherche ou le tri par nom en a besoin
    lower: Vec<Low>,
    lower_ok: bool,
    /// V. Démarrage : RAM actuelle par exe de démarrage (calculée à la mesure, pas à l'image)
    pub startup_ram: HashMap<String, (u64, usize)>,
    pub help: bool,
    pub game_dialog: bool,
    pub game_active: bool,
    pub elevated: bool,
    pub taskmgr: bool,
    /// la copie de Program Files (Ctrl+Maj+Échap) est d'une autre version
    pub taskmgr_stale: Arc<std::sync::atomic::AtomicBool>,
    taskmgr_stale_told: bool,
    pub icons: HashMap<Arc<str>, IconEntry>,
    icon_tx: Sender<Arc<str>>,
    icon_rx: Receiver<(Arc<str>, Option<egui::ColorImage>, Option<String>)>,
    pub actions: Vec<Action>,
    pub session: u32,
    pub self_pid: u32,
    pub last_frame_ms: f32,
    pub minimized: bool,
    /// une app est en plein écran (jeu, vidéo) : on traite les mesures sans dessiner
    pub fullscreen: bool,
    /// mode sans fenêtre (--avant-jeu) : un processus resté inactif pendant toute la fenêtre de mesure
    /// est considéré inactif depuis son lancement
    pub headless: bool,
    /// octets libérés par le dernier mode jeu (sortie JSON de --avant-jeu)
    pub game_freed: u64,
    pub startup: Option<Vec<crate::startup::Entry>>,
    pub docker: crate::docker::Shared,
    docker_running: Arc<std::sync::atomic::AtomicBool>,
    pub journal: Vec<crate::journal::Rec>,
    /// V4 : les 5 coupables de chaque mesure de l'historique
    pub blackbox: VecDeque<crate::qol::Moment>,
    /// V4 : instant épinglé par un clic sur un graphique (ressource, index dans l'anneau)
    pub bb_pin: Option<(crate::qol::Metric, usize)>,
    /// V2 : projets en cours sous `dev_root`
    pub projects: Vec<crate::qol::Project>,
    /// travaux longs en cours sur un thread (nettoyage, mode jeu)
    job_tx: Sender<crate::travail::Fini>,
    job_rx: Receiver<crate::travail::Fini>,
    pub busy: usize,
    peaks_saved: std::time::Instant,
    releve_saved: std::time::Instant,
}

impl App {
    pub fn new(ctx: &egui::Context, s: Settings) -> App {
        theme::install_fonts(ctx);
        let pal = theme::palette(s.dark);
        theme::apply(ctx, &pal);
        ctx.all_styles_mut(|st| {
            st.spacing.item_spacing = egui::vec2(8.0, 6.0);
            st.spacing.button_padding = egui::vec2(10.0, 5.0);
            st.interaction.tooltip_delay = 0.35;
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let c2 = ctx.clone();
        let waker: Waker = Arc::new(std::sync::OnceLock::new());
        let w2 = waker.clone();
        let (job_tx, job_rx) = std::sync::mpsc::channel();
        let docker_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c4 = ctx.clone();
        let docker = crate::docker::spawn(docker_running.clone(), move || c4.request_repaint());
        let cmd = sampler::spawn(Duration::from_millis(s.interval_ms), s.gpu, move |snap| {
            let _ = tx.send(snap);
            // fenêtre réduite : egui ne relaie plus les demandes de dessin, la boucle est réveillée directement
            match w2.get() {
                Some(w) => w(),
                None => c2.request_repaint(),
            }
        });
        // Icônes et descriptions des exe, chargées à la demande hors du thread d'interface.
        let (icon_tx, irx) = std::sync::mpsc::channel::<Arc<str>>();
        let (itx, icon_rx) = std::sync::mpsc::channel();
        let c3 = ctx.clone();
        let large = ctx.pixels_per_point() > 1.3;
        std::thread::Builder::new()
            .name("icones".into())
            .spawn(move || {
                while let Ok(path) = irx.recv() {
                    let img = sys::exe_icon(&path, large).map(|(w, h, px)| egui::ColorImage::from_rgba_unmultiplied([w, h], &px));
                    let desc = crate::winfo::description(&path);
                    if itx.send((path, img, desc)).is_err() {
                        break;
                    }
                    c3.request_repaint_after(Duration::from_millis(30));
                }
            })
            .ok();
        let mut s = s;
        crate::travail::EtatJeu::migrer(s.prev_power.take());
        let game_active = crate::travail::EtatJeu::load().actif;
        if let Ok(v) = std::env::var("VIGIE_VIEW") {
            s.view = match v.as_str() { "hunter" => View::Hunter, "perf" => View::Perf, "startup" => View::Startup, _ => View::Procs };
        }
        App {
            ctx: ctx.clone(),
            pal,
            snap: None,
            rx,
            cmd,
            hist: Hists::default(),
            phist: HashMap::new(),
            rows: Vec::new(),
            rows_dirty: true,
            rank: HashMap::new(),
            freeze: false,
            by_key: HashMap::new(),
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
            scroll_to_cursor: false,
            filter: String::new(),
            focus_search: false,
            collapsed: HashSet::new(),
            ghosts: Vec::new(),
            checked: HashMap::new(),
            toasts: Default::default(),
            confirm: None,
            palette: Default::default(),
            search_ac: socle::autocomplete::Autocomplete::new(s.search_history.clone()),
            search_field: Default::default(),
            cand_fp: 0,
            lower: Vec::new(),
            lower_ok: false,
            startup_ram: HashMap::new(),
            help: false,
            game_dialog: false,
            game_active,
            elevated: sys::is_elevated(),
            taskmgr: sys::taskmgr_replaced(),
            taskmgr_stale: {
                let f = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let f2 = f.clone();
                if !socle::testmode::enabled() {
                    std::thread::Builder::new().name("ifeo".into()).spawn(move || f2.store(sys::taskmgr_copy_stale(), std::sync::atomic::Ordering::Relaxed)).ok();
                }
                f
            },
            taskmgr_stale_told: false,
            icons: HashMap::new(),
            icon_tx,
            icon_rx,
            actions: Vec::new(),
            session: sys::current_session(),
            self_pid: std::process::id(),
            last_frame_ms: 0.0,
            minimized: false,
            fullscreen: false,
            headless: false,
            waker,
            game_freed: 0,
            startup: None,
            docker,
            docker_running,
            journal: crate::journal::load(30),
            blackbox: VecDeque::new(),
            bb_pin: None,
            projects: Vec::new(),
            job_tx,
            job_rx,
            busy: 0,
            peaks_saved: std::time::Instant::now(),
            releve_saved: std::time::Instant::now(),
            s,
        }
    }

    pub fn set_dark(&mut self, d: bool) {
        self.s.dark = d;
        self.pal = theme::palette(d);
        theme::apply(&self.ctx, &self.pal);
    }

    pub fn toast(&mut self, msg: impl Into<String>, err: bool) {
        if err { self.toasts.error(msg) } else { self.toasts.ok(msg) }
        self.ctx.request_repaint();
    }

    /// Dernier message affiché (mode sans fenêtre : résumé imprimé sur stdout).
    pub fn last_toast(&self) -> Option<String> {
        self.toasts.current().map(|t| t.msg.clone())
    }

    pub fn set_background(&mut self, b: bool) {
        if b != self.minimized {
            self.minimized = b;
            let _ = self.cmd.send(Cmd::Background(b));
        }
    }

    pub fn set_interval(&mut self, ms: u64) {
        self.s.interval_ms = ms;
        let _ = self.cmd.send(Cmd::Interval(Duration::from_millis(ms)));
    }

    pub fn set_gpu(&mut self, on: bool) {
        self.s.gpu = on;
        let _ = self.cmd.send(Cmd::Gpu(on));
    }

    pub fn proc(&self, k: Key) -> Option<&Proc> {
        let snap = self.snap.as_ref()?;
        self.by_key.get(&k).and_then(|&i| snap.procs.get(i))
    }

    pub fn icon(&mut self, path: &Arc<str>) -> Option<&IconEntry> {
        if !self.icons.contains_key(path) {
            self.icons.insert(path.clone(), IconEntry { tex: None, desc: None });
            let _ = self.icon_tx.send(path.clone());
        }
        self.icons.get(path)
    }

    /// Réception des mesures et des icônes (appelé à chaque image).
    pub fn tick(&mut self) {
        while let Ok((path, img, desc)) = self.icon_rx.try_recv() {
            let tex = img.map(|im| self.ctx.load_texture(format!("ico:{path}"), im, egui::TextureOptions::LINEAR));
            self.icons.insert(path, IconEntry { tex, desc });
        }
        if !self.taskmgr_stale_told && self.taskmgr_stale.load(std::sync::atomic::Ordering::Relaxed) {
            self.taskmgr_stale_told = true;
            self.toast("Ctrl+Maj+Échap ouvre une ancienne copie de Vigie : Ctrl+K → « Mettre à jour la copie du Gestionnaire »", true);
        }
        while let Ok(f) = self.job_rx.try_recv() {
            self.job_done(f);
        }
        let mut latest = None;
        while let Ok(s) = self.rx.try_recv() {
            latest = Some(s);
        }
        if let Some(snap) = latest {
            self.on_snapshot(snap);
        }
    }

    fn on_snapshot(&mut self, snap: Arc<Snapshot>) {
        self.fullscreen = snap.fullscreen;
        let snap = if self.headless {
            let procs = snap.procs.iter().cloned().map(|mut p| {
                if p.idle_secs >= 1.0 {
                    p.idle_secs = p.age_secs;
                }
                p
            });
            Arc::new(Snapshot { procs: procs.collect(), totals: snap.totals.clone(), cost_ms: snap.cost_ms, ncpu: snap.ncpu, parts: snap.parts.clone(), fullscreen: snap.fullscreen })
        } else {
            snap
        };
        let t = &snap.totals;
        self.hist.cpu.push(t.cpu);
        self.hist.mem.push(t.mem_used as f32 / t.mem_total.max(1) as f32 * 100.0);
        self.hist.gpu.push(t.gpu);
        self.hist.disk.push((t.disk_read + t.disk_write) as f32);
        self.hist.net.push((t.net_rx + t.net_tx) as f32);
        self.by_key = snap.procs.iter().enumerate().map(|(i, p)| (p.key, i)).collect();
        for p in &snap.procs {
            let e = self.phist.entry(p.key).or_default();
            e.0.push(p.cpu);
            e.1.push(p.mem as f32);
        }
        self.phist.retain(|k, _| self.by_key.contains_key(k));
        self.selected.retain(|k| self.by_key.contains_key(k));
        self.collapsed.retain(|k| self.by_key.contains_key(k));
        if self.cursor.map(|k| !self.by_key.contains_key(&k)).unwrap_or(false) {
            self.cursor = None;
        }
        let docker_on = snap.procs.iter().any(|p| p.name.eq_ignore_ascii_case("com.docker.backend.exe"));
        self.docker_running.store(docker_on, std::sync::atomic::Ordering::Relaxed);
        let containers = if docker_on { self.docker.lock().map(|d| d.containers.clone()).unwrap_or_default() } else { Vec::new() };
        let cx = hunter::Context { session: self.session, self_pid: self.self_pid, whitelist: &self.s.whitelist, docker: &containers };
        self.ghosts = hunter::hunt(&snap, &cx);
        let alive: HashSet<Key> = self.ghosts.iter().map(|g| g.key).collect();
        self.checked.retain(|k, _| alive.contains(k));
        for g in &self.ghosts {
            self.checked.entry(g.key).or_insert(g.safe);
        }
        // Debug (captures) : VIGIE_SELECT=nom.exe sélectionne ce processus, VIGIE_PALETTE=texte ouvre la palette.
        if self.cursor.is_none() {
            if let Ok(n) = std::env::var("VIGIE_SELECT") {
                if let Some(p) = snap.procs.iter().filter(|p| p.name.eq_ignore_ascii_case(&n)).max_by_key(|p| p.mem) {
                    self.cursor = Some(p.key);
                    self.selected = [p.key].into_iter().collect();
                }
            }
        }
        if !self.palette.is_open() && self.hist.cpu.0.len() == 2 {
            if std::env::var_os("VIGIE_GAME").is_some() {
                self.game_dialog = true;
            }
            if std::env::var_os("VIGIE_BB").is_some() {
                self.bb_pin = Some((crate::qol::Metric::Mem, 0));
            }
            if let Some(q) = std::env::var_os("VIGIE_PALETTE") {
                // socle 0.1.3 : saisie pré-remplie (« 1 » = palette vide)
                let q = q.to_string_lossy().into_owned();
                if q == "1" { self.palette.open() } else { self.palette.open_with(q) }
            }
        }
        // V4 boîte noire, V2 projets, V3 pics des jeux (tout depuis l'instantané, ~µs)
        if self.bb_pin.is_some() && self.blackbox.len() >= HIST {
            // l'anneau glisse : l'instant épinglé recule d'un cran
            self.bb_pin = self.bb_pin.and_then(|(m, i)| i.checked_sub(1).map(|i| (m, i)));
        }
        crate::qol::push_moment(&mut self.blackbox, crate::qol::moment(&snap), HIST);
        let untouchable = crate::qol::self_and_ancestors(&snap, self.self_pid);
        self.projects = crate::qol::projects(&snap, self.session, &self.s.dev_root, &untouchable);
        let icons = &self.icons;
        let desc = |path: &str| icons.get(path).and_then(|e| e.desc.clone()).filter(|d| !d.is_empty());
        if crate::qol::track_games(&snap, &mut self.s.game_peaks, desc) && self.peaks_saved.elapsed() > Duration::from_secs(30) && !self.headless {
            self.s.save();
            self.peaks_saved = std::time::Instant::now();
        }
        // candidats d'autocomplétion : reconstruits seulement si les processus (ou les descriptions
        // connues, ou les ports) ont changé
        let mut fp: u64 = self.icons.len() as u64;
        for p in &snap.procs {
            fp = fp.wrapping_mul(31).wrapping_add(p.key.pid as u64 ^ p.key.ctime as u64 ^ p.ports.len() as u64);
        }
        if fp != self.cand_fp && !self.headless && !self.minimized {
            self.cand_fp = fp;
            let mut cands: Vec<String> = Vec::with_capacity(snap.procs.len() + 32);
            for p in &snap.procs {
                cands.push(p.name.to_string());
                if let Some(d) = p.path.as_ref().and_then(|pa| self.icons.get(pa)).and_then(|e| e.desc.clone()) {
                    cands.push(d);
                }
                cands.extend(p.ports.iter().filter(|x| x.tcp).map(|x| format!(":{}", x.port)));
            }
            cands.sort();
            cands.dedup();
            self.search_ac.set_candidates(cands);
        }
        if self.startup.is_some() && self.s.view == View::Startup {
            self.startup_ram = crate::startup::ram_table(self.startup.as_deref().unwrap_or(&[]), &snap.procs);
        }
        // relevé d'inactivité toutes les 5 min (et à la fermeture) : survit à un plantage
        if !self.headless && self.releve_saved.elapsed() > Duration::from_secs(300) {
            crate::journal::Releve::save(&snap.procs);
            self.releve_saved = std::time::Instant::now();
        }
        self.lower_ok = false;
        self.snap = Some(snap);
        self.rows_dirty = true;
    }

    pub fn sample_now(&self) {
        let _ = self.cmd.send(Cmd::Now);
    }

    // ---------- Lignes affichées ----------
    pub fn rebuild_rows(&mut self) {
        self.rows_dirty = false;
        let Some(snap) = self.snap.clone() else { return };
        let procs = &snap.procs;
        let q = self.filter.trim().to_lowercase();
        let visible = |p: &Proc| !(self.s.hide_system && (p.session == 0 || p.key.pid <= 4));
        if (!q.is_empty() || self.s.sort == Col::Name) && !self.lower_ok {
            self.lower = procs.iter().map(|p| Low::of(p, &self.icons)).collect();
            self.lower_ok = true;
        }
        let lower = &self.lower;
        let by_key = &self.by_key;
        let low = |p: &Proc| by_key.get(&p.key).and_then(|&i| lower.get(i));
        let matches = |p: &Proc| -> bool {
            if q.is_empty() {
                return true;
            }
            if let Some(port) = q.strip_prefix(':') {
                return p.ports.iter().any(|x| x.port.to_string().starts_with(port));
            }
            let Some(l) = low(p) else { return false };
            q.split_whitespace().all(|w| {
                socle::fuzzy::score(&l.name, w).is_some()
                    || (!l.desc.is_empty() && (socle::fuzzy::score(&l.desc, w).is_some() || l.desc.contains(w)))
                    || p.key.pid.to_string() == w
                    || l.title.contains(w)
                    || (w.len() >= 3 && l.cmd.contains(w))
            })
        };
        let sort = self.s.sort;
        let asc = self.s.asc;
        let freeze = self.freeze && !self.rank.is_empty();
        let rank = &self.rank;
        let cmp = |a: &Proc, b: &Proc, aa: Option<&Agg>, ba: Option<&Agg>| -> std::cmp::Ordering {
            use std::cmp::Ordering;
            if freeze {
                let ra = rank.get(&a.key).copied().unwrap_or(usize::MAX);
                let rb = rank.get(&b.key).copied().unwrap_or(usize::MAX);
                return ra.cmp(&rb).then(a.key.pid.cmp(&b.key.pid));
            }
            let num = |p: &Proc, g: Option<&Agg>| -> f64 {
                match sort {
                    Col::Cpu => g.map(|g| g.cpu).unwrap_or(p.cpu) as f64,
                    Col::Mem => g.map(|g| g.mem).unwrap_or(p.mem) as f64,
                    Col::Disk => g.map(|g| g.disk).unwrap_or(p.disk),
                    Col::Gpu => g.map(|g| g.gpu).unwrap_or(p.gpu) as f64,
                    Col::GpuMem => g.map(|g| g.gpu_mem).unwrap_or(p.gpu_mem) as f64,
                    Col::Pid => p.key.pid as f64,
                    Col::Threads => p.threads as f64,
                    Col::Ports => p.ports.len() as f64,
                    Col::Status => (p.suspended as u8 * 2 + p.window.is_some() as u8) as f64,
                    Col::Name => 0.0,
                }
            };
            let o = if sort == Col::Name {
                low(a).map(|l| l.name.as_str()).cmp(&low(b).map(|l| l.name.as_str()))
            } else {
                num(a, aa).partial_cmp(&num(b, ba)).unwrap_or(Ordering::Equal)
            };
            let o = if asc { o } else { o.reverse() };
            o.then(a.name.cmp(&b.name)).then(a.key.pid.cmp(&b.key.pid))
        };
        let mut rows = Vec::with_capacity(procs.len());
        if !self.s.tree {
            let mut idx: Vec<usize> = (0..procs.len()).filter(|&i| visible(&procs[i]) && matches(&procs[i])).collect();
            idx.sort_by(|&a, &b| cmp(&procs[a], &procs[b], None, None));
            rows.extend(idx.into_iter().map(|i| Row { idx: i, depth: 0, has_kids: false, open: false, agg: None }));
        } else {
            let mut kids: HashMap<Key, Vec<usize>> = HashMap::new();
            let mut roots = Vec::new();
            for (i, p) in procs.iter().enumerate() {
                if !visible(p) {
                    continue;
                }
                match p.parent.filter(|k| self.by_key.get(k).map(|&j| visible(&procs[j])).unwrap_or(false)) {
                    Some(par) => kids.entry(par).or_default().push(i),
                    None => roots.push(i),
                }
            }
            // agrégats + filtre (un nœud est gardé si lui ou un descendant correspond)
            let mut agg: HashMap<Key, Agg> = HashMap::new();
            let mut keep: HashSet<Key> = HashSet::new();
            fn walk(i: usize, procs: &[Proc], kids: &HashMap<Key, Vec<usize>>, agg: &mut HashMap<Key, Agg>, keep: &mut HashSet<Key>, m: &dyn Fn(&Proc) -> bool, depth: u32) -> (Agg, bool) {
                let p = &procs[i];
                let mut a = Agg { cpu: p.cpu, mem: p.mem, disk: p.disk, gpu: p.gpu, gpu_mem: p.gpu_mem, count: 1 };
                let mut any = m(p);
                if depth < 64 {
                    if let Some(ch) = kids.get(&p.key) {
                        for &c in ch {
                            let (ca, ck) = walk(c, procs, kids, agg, keep, m, depth + 1);
                            a.cpu += ca.cpu;
                            a.mem += ca.mem;
                            a.disk += ca.disk;
                            a.gpu += ca.gpu;
                            a.gpu_mem += ca.gpu_mem;
                            a.count += ca.count;
                            any |= ck;
                        }
                    }
                }
                agg.insert(p.key, a);
                if any {
                    keep.insert(p.key);
                }
                (a, any)
            }
            for &r in &roots {
                walk(r, procs, &kids, &mut agg, &mut keep, &matches, 0);
            }
            let filtering = !q.is_empty();
            let sort_list = |v: &mut Vec<usize>| v.sort_by(|&a, &b| cmp(&procs[a], &procs[b], agg.get(&procs[a].key), agg.get(&procs[b].key)));
            sort_list(&mut roots);
            let mut stack: Vec<(usize, u16)> = roots.iter().rev().filter(|&&i| keep.contains(&procs[i].key)).map(|&i| (i, 0)).collect();
            while let Some((i, d)) = stack.pop() {
                let p = &procs[i];
                let mut ch: Vec<usize> = kids.get(&p.key).cloned().unwrap_or_default();
                ch.retain(|&c| keep.contains(&procs[c].key));
                let has_kids = !ch.is_empty();
                let open = has_kids && (filtering || !self.collapsed.contains(&p.key));
                rows.push(Row { idx: i, depth: d, has_kids, open, agg: (has_kids && !open).then(|| agg[&p.key]) });
                if open && d < 60 {
                    sort_list(&mut ch);
                    for &c in ch.iter().rev() {
                        stack.push((c, d + 1));
                    }
                }
            }
        }
        if !self.freeze {
            self.rank = rows.iter().enumerate().map(|(n, r)| (procs[r.idx].key, n)).collect();
        }
        self.rows = rows;
    }

    pub fn row_keys(&self) -> Vec<Key> {
        let Some(snap) = &self.snap else { return Vec::new() };
        self.rows.iter().map(|r| snap.procs[r.idx].key).collect()
    }

    /// Sélection effective pour les actions : la sélection, sinon la ligne sous le curseur.
    pub fn targets(&self) -> Vec<Key> {
        if !self.selected.is_empty() {
            let mut v: Vec<Key> = self.selected.iter().copied().collect();
            v.sort();
            v
        } else {
            self.cursor.into_iter().collect()
        }
    }

    pub fn move_cursor(&mut self, delta: i32, extend: bool) {
        let keys = self.row_keys();
        if keys.is_empty() {
            return;
        }
        let cur = self.cursor.and_then(|c| keys.iter().position(|k| *k == c));
        let n = keys.len() as i32;
        let next = match cur {
            None => if delta >= 0 { 0 } else { n - 1 },
            Some(i) => (i as i32 + delta).clamp(0, n - 1),
        } as usize;
        let k = keys[next];
        self.cursor = Some(k);
        if extend {
            let a = self.anchor.and_then(|a| keys.iter().position(|x| *x == a)).unwrap_or(next);
            let (lo, hi) = (a.min(next), a.max(next));
            self.selected = keys[lo..=hi].iter().copied().collect();
        } else {
            self.anchor = Some(k);
            self.selected = [k].into_iter().collect();
        }
        self.scroll_to_cursor = true;
    }

    pub fn select_key(&mut self, k: Key) {
        // si la cible est dans un sous-arbre replié, on déplie ses ancêtres
        let mut cur = self.proc(k).and_then(|p| p.parent);
        while let Some(par) = cur {
            self.collapsed.remove(&par);
            cur = self.proc(par).and_then(|p| p.parent);
        }
        self.filter.clear();
        self.s.view = View::Procs;
        self.cursor = Some(k);
        self.anchor = Some(k);
        self.selected = [k].into_iter().collect();
        self.scroll_to_cursor = true;
        self.rows_dirty = true;
    }

    // ---------- Actions ----------
    pub fn apply_actions(&mut self) {
        let acts = std::mem::take(&mut self.actions);
        for a in acts {
            self.apply(a);
        }
    }

    fn names(&self, keys: &[Key]) -> String {
        let mut v: Vec<String> = keys.iter().filter_map(|k| self.proc(*k)).map(|p| format!("{} ({})", p.name, p.key.pid)).collect();
        if v.len() > 4 {
            let n = v.len() - 4;
            v.truncate(4);
            v.push(format!("et {n} autre(s)"));
        }
        v.join(", ")
    }

    fn apply(&mut self, a: Action) {
        match a {
            Action::Kill(keys, tree) => {
                if keys.is_empty() {
                    return;
                }
                if self.s.confirm_kill {
                    let names = self.names(&keys);
                    self.confirm = Some(Confirm::Kill { keys, tree, names });
                } else {
                    self.apply(Action::KillNow(keys, tree));
                }
            }
            Action::KillNow(keys, tree) => {
                let (ok, errs, freed) = self.kill_keys(&keys, tree);
                if errs.is_empty() {
                    self.toast(format!("{ok} processus terminé(s) · {} libérés", human_bytes(freed)), false);
                } else {
                    self.toast(format!("{ok} terminé(s), {} échec(s) : {}", errs.len(), errs[0]), true);
                }
                self.selected.clear();
                self.sample_now();
            }
            Action::Close(keys) => {
                let n = keys.iter().filter(|k| sys::close_gracefully(k.pid)).count();
                if n == 0 {
                    self.toast("Aucune fenêtre à fermer (utilise « Terminer »)", true);
                } else {
                    self.toast(format!("Fermeture demandée à {n} processus"), false);
                }
                self.sample_now();
            }
            Action::Suspend(keys, on) => {
                let mut err = None;
                // déjà dans l'état demandé : on saute (les suspensions s'empileraient)
                let now = sys::suspended_now();
                let keys: Vec<Key> = keys.into_iter().filter(|k| now.contains(&(k.pid, k.ctime)) != on).collect();
                for k in &keys {
                    if let Err(e) = sys::suspend(k.pid, k.ctime, on) {
                        err = Some(e);
                    }
                }
                match err {
                    Some(e) => self.toast(e, true),
                    None => self.toast(if on { "Suspendu (Espace pour reprendre)" } else { "Repris" }, false),
                }
                self.sample_now();
            }
            Action::Priority(keys, p) => {
                let mut err = None;
                for k in &keys {
                    if let Err(e) = sys::set_priority(k.pid, k.ctime, p) {
                        err = Some(e);
                    }
                }
                match err {
                    Some(e) => self.toast(e, true),
                    None => self.toast(format!("Priorité : {}", p.label()), false),
                }
                self.sample_now();
            }
            Action::Focus(k) => {
                if !sys::focus_window_of(k.pid) {
                    self.toast("Ce processus n’a pas de fenêtre", true);
                }
            }
            Action::OpenLocation(k) => match self.proc(k).and_then(|p| p.path.clone()) {
                Some(p) => sys::open_location(&p),
                None => self.toast("Emplacement inconnu (processus protégé ?)", true),
            },
            Action::Copy(s) => {
                self.ctx.copy_text(s);
                self.toast("Copié", false);
            }
            Action::Select(k) => self.select_key(k),
            Action::Clean(keys) => {
                let ghosts: Vec<Ghost> = self.ghosts.iter().filter(|g| keys.contains(&g.key)).cloned().collect();
                let plan = self.plan_chasses(&ghosts);
                self.toast(format!("Nettoyage en cours ({} fantôme(s))…", plan.len()), false);
                self.spawn_job(crate::travail::Travail::Nettoyer(plan));
            }
            Action::Whitelist(rule) => {
                if !self.s.whitelist.contains(&rule) {
                    self.s.whitelist.push(rule.clone());
                    self.s.save();
                }
                self.toast(format!("« {rule} » ne sera plus proposé"), false);
                self.sample_now();
            }
            Action::Unwhitelist(rule) => {
                self.s.whitelist.retain(|r| r != &rule);
                self.s.save();
                self.sample_now();
            }
            Action::GameMode => self.game_dialog = true,
            Action::GameExit => self.exit_game_mode(),
            Action::Elevate => {
                self.s.save();
                if sys::run_elevated("--after-elevate", false) {
                    std::process::exit(0);
                } else {
                    self.toast("Élévation annulée", true);
                }
            }
            Action::TaskmgrToggle => {
                let on = !self.taskmgr;
                match sys::set_taskmgr(on) {
                    Ok(()) => {
                        self.taskmgr = on;
                        self.toast(if on { "Ctrl+Maj+Échap ouvre maintenant Vigie" } else { "Gestionnaire des tâches Windows rétabli" }, false);
                    }
                    Err(e) => self.toast(e, true),
                }
            }
            Action::TaskmgrUpdate => match sys::set_taskmgr(true) {
                Ok(()) => {
                    self.taskmgr_stale.store(false, std::sync::atomic::Ordering::Relaxed);
                    self.toast("Copie du Gestionnaire des tâches mise à jour", false);
                }
                Err(e) => self.toast(e, true),
            },
            Action::SampleNow => self.sample_now(),
            Action::SetView(v) => self.s.view = v,
            Action::Trim(keys) => {
                let (n, freed, errs) = self.trim_keys(&keys);
                if errs.is_empty() || n > 0 {
                    self.toast(format!("{n} processus mis en veille · {} libérés (ils restent ouverts)", human_bytes(freed)), false);
                } else {
                    self.toast(errs[0].clone(), true);
                }
                self.sample_now();
            }
            Action::KillPort(port) => {
                let keys: Vec<Key> = self.snap.as_ref().map(|s| s.procs.iter().filter(|p| p.ports.iter().any(|x| x.tcp && x.port == port)).map(|p| p.key).collect()).unwrap_or_default();
                if keys.is_empty() {
                    self.toast(format!("Rien n’écoute sur :{port}"), true);
                } else {
                    self.apply(Action::Kill(keys, true));
                }
            }
            Action::SleepProject(dir) => {
                let Some(pr) = self.projects.iter().find(|p| p.dir == dir).cloned() else { return };
                let (mut n, mut freed, mut errs) = (0, 0u64, Vec::new());
                let now = sys::suspended_now();
                for k in &pr.keys {
                    // déjà endormi : on ne suspend pas une 2e fois
                    if now.contains(&(k.pid, k.ctime)) {
                        continue;
                    }
                    match sys::suspend(k.pid, k.ctime, true) {
                        Ok(()) => {
                            n += 1;
                            freed += sys::trim(k.pid, k.ctime).unwrap_or(0);
                        }
                        Err(e) => errs.push(e),
                    }
                }
                if n > 0 {
                    self.log(vec![crate::journal::Rec { t: crate::journal::now(), what: "sieste".into(), name: pr.name.clone(), bytes: freed }]);
                    self.toast(format!("Projet « {} » endormi : {n} processus suspendus, {} libérés", pr.name, human_bytes(freed)), false);
                } else {
                    self.toast(errs.first().cloned().unwrap_or_else(|| "rien à endormir".into()), true);
                }
                self.sample_now();
            }
            Action::WakeProject(dir) => {
                let Some(pr) = self.projects.iter().find(|p| p.dir == dir).cloned() else { return };
                let mut n = 0;
                let now = sys::suspended_now();
                for k in &pr.keys {
                    // seulement ceux qui sont suspendus (état réel, pas la dernière mesure)
                    if !now.contains(&(k.pid, k.ctime)) {
                        continue;
                    }
                    if sys::suspend(k.pid, k.ctime, false).is_ok() {
                        n += 1;
                        let _ = sys::prefetch(k.pid, k.ctime);
                    }
                }
                self.toast(format!("Projet « {} » réveillé : {n} processus", pr.name), false);
                self.sample_now();
            }
            Action::StartupToggle(i, on) => {
                if let Some(e) = self.startup.as_ref().and_then(|v| v.get(i)).cloned() {
                    match crate::startup::set_enabled(&e, on) {
                        Ok(()) => {
                            self.toast(format!("« {} » {} au démarrage", e.name, if on { "activé" } else { "désactivé" }), false);
                            self.startup = Some(crate::startup::list());
                        }
                        Err(err) => self.toast(err, true),
                    }
                }
            }
            Action::StartupRefresh => self.startup = Some(crate::startup::list()),
            Action::WebSearch(name) => {
                let q: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '+' }).collect();
                let _ = std::process::Command::new("explorer.exe").arg(format!("https://www.google.com/search?q={q}+processus+windows")).spawn();
            }
        }
    }

    fn kill_keys(&mut self, keys: &[Key], tree: bool) -> (usize, Vec<String>, u64) {
        let names: Vec<String> = keys.iter().filter_map(|k| self.proc(*k)).map(|p| p.name.to_string()).collect();
        let r = self.kill_keys_quiet(keys, tree);
        if r.0 > 0 {
            let name = if names.len() == 1 { names[0].clone() } else { format!("{} processus", names.len()) };
            self.log(vec![crate::journal::Rec { t: crate::journal::now(), what: "tué".into(), name, bytes: r.2 }]);
        }
        r
    }

    fn kill_keys_quiet(&mut self, keys: &[Key], tree: bool) -> (usize, Vec<String>, u64) {
        let Some(snap) = self.snap.clone() else { return (0, vec![], 0) };
        let mut order: Vec<Key> = Vec::new();
        for &k in keys {
            if tree {
                order.extend(hunter::kill_order(&snap.procs, k));
            } else {
                order.push(k);
            }
        }
        let mut seen = HashSet::new();
        order.retain(|k| seen.insert(*k));
        let (mut ok, mut errs, mut freed) = (0, Vec::new(), 0u64);
        for k in order {
            let Some(p) = self.proc(k) else { continue };
            let n = p.name.to_ascii_lowercase();
            if CRITICAL.contains(&n.as_str()) || k.pid <= 4 {
                errs.push(format!("{} est vital pour Windows", p.name));
                continue;
            }
            if k.pid == self.self_pid {
                continue;
            }
            let mem = p.mem;
            match sys::kill(k.pid, k.ctime) {
                Ok(()) => {
                    ok += 1;
                    freed += mem;
                }
                Err(e) => errs.push(format!("{} : {e}", p.name)),
            }
        }
        (ok, errs, freed)
    }

    /// Cibles d'un arbre (enfants d'abord), sans les processus vitaux ni Vigie.
    fn cibles(&self, roots: &[Key]) -> Vec<crate::travail::Cible> {
        let Some(snap) = &self.snap else { return Vec::new() };
        let mut order: Vec<Key> = Vec::new();
        for &r in roots {
            order.extend(hunter::kill_order(&snap.procs, r));
        }
        let mut seen = HashSet::new();
        order.retain(|k| seen.insert(*k));
        order
            .into_iter()
            .filter_map(|k| self.proc(k))
            .filter(|p| p.key.pid > 4 && p.key.pid != self.self_pid && !CRITICAL.contains(&p.name.to_ascii_lowercase().as_str()))
            .map(|p| crate::travail::Cible { key: p.key, name: p.name.to_string(), mem: p.mem })
            .collect()
    }

    /// Plan de chasse (exécuté sur un thread par `travail`).
    pub fn plan_chasses(&self, ghosts: &[Ghost]) -> Vec<crate::travail::Chasse> {
        use crate::travail::{Chasse, Remede};
        ghosts
            .iter()
            .map(|g| Chasse {
                name: g.name.clone(),
                ram: g.ram,
                remede: match &g.fix {
                    Fix::KillTree => Remede::Tuer(self.cibles(&[g.key])),
                    Fix::WslShutdown => Remede::WslShutdown,
                    Fix::DockerStop(id) => Remede::DockerStop(id.clone()),
                    Fix::DockerQuit => {
                        let roots: Vec<Key> = self
                            .snap
                            .as_ref()
                            .map(|s| s.procs.iter().filter(|p| hunter::is_docker_name(&p.name.to_ascii_lowercase())).map(|p| p.key).collect())
                            .unwrap_or_default();
                        Remede::DockerQuit(self.cibles(&roots))
                    }
                },
            })
            .collect()
    }

    pub fn spawn_job(&mut self, t: crate::travail::Travail) {
        self.busy += 1;
        let tx = self.job_tx.clone();
        let waker = self.waker.clone();
        let ctx = self.ctx.clone();
        std::thread::Builder::new()
            .name("travail".into())
            .spawn(move || {
                let _ = tx.send(crate::travail::executer(t));
                match waker.get() {
                    Some(w) => w(),
                    None => ctx.request_repaint(),
                }
            })
            .ok();
    }

    fn job_done(&mut self, f: crate::travail::Fini) {
        self.busy = self.busy.saturating_sub(1);
        self.log(f.journal);
        if f.jeu {
            self.game_active = true;
            self.game_freed += f.libere;
        }
        if f.message.starts_with("Mode jeu terminé") {
            self.game_active = false;
        }
        self.toast(f.message, f.erreur);
        self.sample_now();
    }

    /// Mode sans fenêtre : attend la fin des travaux lancés.
    pub fn wait_jobs(&mut self, timeout: Duration) {
        let t0 = std::time::Instant::now();
        while self.busy > 0 && t0.elapsed() < timeout {
            match self.job_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(f) => self.job_done(f),
                Err(_) => continue,
            }
        }
    }

    pub fn log(&mut self, recs: Vec<crate::journal::Rec>) {
        if recs.is_empty() {
            return;
        }
        crate::journal::append(&recs);
        self.journal.extend(recs);
    }

    /// Vide la RAM des processus sans les fermer.
    pub fn trim_keys(&mut self, keys: &[Key]) -> (usize, u64, Vec<String>) {
        let cibles: Vec<crate::travail::Cible> = keys.iter().filter_map(|k| self.proc(*k)).map(|p| crate::travail::Cible { key: p.key, name: p.name.to_string(), mem: p.mem }).collect();
        let (n, freed, recs) = crate::travail::endormir(&cibles);
        self.log(recs);
        let errs = if n == 0 && !cibles.is_empty() { vec!["accès refusé (processus protégé ?)".to_string()] } else { Vec::new() };
        (n, freed, errs)
    }

    /// Apps en arrière-plan à mettre en veille avant de jouer : processus de l'utilisateur > 64 Mo,
    /// hors Windows, hors protégés (Discord, Steam…), hors fenêtre au premier plan et hors Vigie.
    pub fn trim_candidates(&self) -> Vec<Key> {
        let Some(snap) = &self.snap else { return Vec::new() };
        let fg = sys::foreground_pid();
        snap.procs
            .iter()
            .filter(|p| p.session == self.session && p.key.pid > 4 && p.key.pid != self.self_pid && p.key.pid != fg)
            .filter(|p| p.ws > 64 << 20)
            .filter(|p| {
                let n = p.name.to_ascii_lowercase();
                !hunter::is_protected_name(&n) && !CRITICAL.contains(&n.as_str()) && !self.s.whitelist.iter().any(|r| hunter::matches_rule(r, &p.name, p.cmd.as_deref()))
            })
            .map(|p| p.key)
            .collect()
    }

    /// V3 : dernier jeu vu (nom, pic, RAM disponible maintenant).
    pub fn last_game(&self) -> Option<(String, u64, u64)> {
        let g = self.s.game_peaks.values().max_by_key(|g| g.seen)?;
        let avail = self.snap.as_ref().map(|s| s.totals.mem_total - s.totals.mem_used)?;
        Some((g.name.clone(), g.peak, avail))
    }

    // ---------- Mode avant de jouer ----------
    pub fn safe_ghosts(&self) -> Vec<Ghost> {
        self.ghosts.iter().filter(|g| g.safe).cloned().collect()
    }

    /// Mode avant de jouer : le plan est préparé ici, exécuté sur un thread (aucun gel).
    pub fn run_game_mode(&mut self) {
        let plan = crate::travail::PlanJeu {
            chasses: if self.s.game_kill { self.plan_chasses(&self.safe_ghosts()) } else { Vec::new() },
            veille: if self.s.game_trim {
                self.trim_candidates().into_iter().filter_map(|k| self.proc(k)).map(|p| crate::travail::Cible { key: p.key, name: p.name.to_string(), mem: p.mem }).collect()
            } else {
                Vec::new()
            },
            purge: self.s.game_purge && self.elevated,
            power: self.s.game_power,
        };
        self.game_active = true;
        self.toast("Mode jeu en cours…", false);
        self.spawn_job(crate::travail::Travail::Jeu(plan));
    }

    pub fn exit_game_mode(&mut self) {
        self.spawn_job(crate::travail::Travail::Finir);
    }
}

/// Les tuer provoque un écran bleu ou une déconnexion.
pub const CRITICAL: &[&str] = &["system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "memory compression", "secure system", "lsaiso.exe"];

pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    let (v, u) = if b >= 1024.0 * 1024.0 * 1024.0 {
        (b / (1024.0 * 1024.0 * 1024.0), "Go")
    } else if b >= 1024.0 * 1024.0 {
        (b / (1024.0 * 1024.0), "Mo")
    } else if b >= 1024.0 {
        (b / 1024.0, "Ko")
    } else {
        return format!("{} o", b as u64);
    };
    let s = if v >= 100.0 { format!("{v:.0}") } else if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    format!("{} {u}", s.replace('.', ","))
}

pub fn human_rate(b: f64) -> String {
    if b < 1024.0 * 0.1 {
        return "0 Mo/s".into();
    }
    format!("{}/s", human_bytes(b as u64))
}

pub fn pct(v: f32) -> String {
    if v < 0.05 { "0 %".into() } else if v < 10.0 { format!("{v:.1} %").replace('.', ",") } else { format!("{v:.0} %") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampler::Totals;

    /// Objectif DIRECTION.md : recherche < 5 ms. 400 faux processus, frappe de « node », puis « :51 ».
    #[test]
    fn recherche_rapide() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Settings::default());
        let procs: Vec<Proc> = (0..400u32)
            .map(|i| Proc {
                key: Key { pid: 1000 + i, ctime: i as i64 },
                ppid: if i % 7 == 0 { 0 } else { 1000 + i - 1 },
                parent: if i % 7 == 0 { None } else { Some(Key { pid: 999 + i, ctime: i as i64 - 1 }) },
                name: Arc::from(format!("{}.exe", ["node", "chrome", "code", "python", "svchost"][i as usize % 5]).as_str()),
                path: None,
                cmd: Some(Arc::from(format!(r"C:\Dev\projet{}\serveur.js --port {}", i % 9, 5000 + i).as_str())),
                session: 1,
                threads: 8,
                handles: 100,
                base_prio: 8,
                suspended: false,
                cpu: (i % 13) as f32,
                cpu_time: 0,
                mem: (i as u64) << 20,
                commit: 0,
                peak_ws: 0,
                ws: 0,
                disk: 0.0,
                gpu: 0.0,
                gpu_mem: 0,
                window: None,
                ports: Arc::from(vec![crate::sys::Port { tcp: true, port: 5000 + i as u16, local_only: true }]),
                idle_secs: 0.0,
                age_secs: 0.0,
                idle_min: false,
            })
            .collect();
        app.on_snapshot(Arc::new(Snapshot { procs, totals: Totals::default(), cost_ms: 0.0, ncpu: 8, parts: String::new(), fullscreen: false }));
        for q in ["n", "no", "nod", "node", ":51"] {
            app.filter = q.into();
            let t0 = std::time::Instant::now();
            app.rebuild_rows();
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            eprintln!("recherche « {q} » : {ms:.2} ms, {} lignes", app.rows.len());
            assert!(ms < 5.0, "« {q} » : {ms:.2} ms");
        }
    }
}
