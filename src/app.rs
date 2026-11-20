//! État de l'application : réglages, mesures reçues, lignes affichées, actions.
use crate::hunter::{self, Fix, Ghost};
use crate::sampler::{self, Cmd, Key, Proc, Snapshot};
use crate::sys::{self, Prio};
use crate::theme::{self, Palette};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

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
    /// plan d'alimentation à restaurer en quittant le mode jeu
    pub prev_power: Option<String>,
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
        }
    }
}

impl Settings {
    /// `%APPDATA%\Vigie`, ou `VIGIE_DATA_DIR` (tests : ne jamais toucher aux réglages ni au journal de l'utilisateur).
    pub fn dir() -> Option<std::path::PathBuf> {
        if let Some(d) = std::env::var_os("VIGIE_DATA_DIR") {
            return Some(std::path::PathBuf::from(d));
        }
        dirs::config_dir().map(|d| d.join("Vigie"))
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

pub struct Palette2 {
    pub query: String,
    pub sel: usize,
    pub just_opened: bool,
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
    SampleNow,
    SetView(View),
    WebSearch(String),
    Trim(Vec<Key>),
    StartupToggle(usize, bool),
    StartupRefresh,
}

pub struct IconEntry {
    pub tex: Option<egui::TextureHandle>,
    pub desc: Option<String>,
}

pub struct App {
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
    pub toast: Option<(String, Instant, bool)>,
    pub confirm: Option<Confirm>,
    pub palette: Option<Palette2>,
    pub help: bool,
    pub game_dialog: bool,
    pub game_active: bool,
    pub elevated: bool,
    pub taskmgr: bool,
    pub icons: HashMap<Arc<str>, IconEntry>,
    icon_tx: Sender<Arc<str>>,
    icon_rx: Receiver<(Arc<str>, Option<egui::ColorImage>, Option<String>)>,
    pub actions: Vec<Action>,
    pub session: u32,
    pub self_pid: u32,
    pub last_frame_ms: f32,
    pub minimized: bool,
    /// mode sans fenêtre (--avant-jeu) : un processus resté inactif pendant toute la fenêtre de mesure
    /// est considéré inactif depuis son lancement
    pub headless: bool,
    /// octets libérés par le dernier mode jeu (sortie JSON de --avant-jeu)
    pub game_freed: u64,
    pub startup: Option<Vec<crate::startup::Entry>>,
    pub docker: crate::docker::Shared,
    docker_running: Arc<std::sync::atomic::AtomicBool>,
    pub journal: Vec<crate::journal::Rec>,
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
        let docker_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c4 = ctx.clone();
        let docker = crate::docker::spawn(docker_running.clone(), move || c4.request_repaint());
        let cmd = sampler::spawn(Duration::from_millis(s.interval_ms), s.gpu, move |snap| {
            let _ = tx.send(snap);
            c2.request_repaint();
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
        let game_active = s.prev_power.is_some();
        let mut s = s;
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
            toast: None,
            confirm: None,
            palette: None,
            help: false,
            game_dialog: false,
            game_active,
            elevated: sys::is_elevated(),
            taskmgr: sys::taskmgr_replaced(),
            icons: HashMap::new(),
            icon_tx,
            icon_rx,
            actions: Vec::new(),
            session: sys::current_session(),
            self_pid: std::process::id(),
            last_frame_ms: 0.0,
            minimized: false,
            headless: false,
            game_freed: 0,
            startup: None,
            docker,
            docker_running,
            journal: crate::journal::load(30),
            s,
        }
    }

    pub fn set_dark(&mut self, d: bool) {
        self.s.dark = d;
        self.pal = theme::palette(d);
        theme::apply(&self.ctx, &self.pal);
    }

    pub fn toast(&mut self, msg: impl Into<String>, err: bool) {
        self.toast = Some((msg.into(), Instant::now(), err));
        self.ctx.request_repaint_after(Duration::from_secs(5));
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
        let mut latest = None;
        while let Ok(s) = self.rx.try_recv() {
            latest = Some(s);
        }
        if let Some(snap) = latest {
            self.on_snapshot(snap);
        }
        if let Some((_, t, _)) = &self.toast {
            if t.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            }
        }
    }

    fn on_snapshot(&mut self, snap: Arc<Snapshot>) {
        let snap = if self.headless {
            let procs = snap.procs.iter().cloned().map(|mut p| {
                if p.idle_secs >= 1.0 {
                    p.idle_secs = p.age_secs;
                }
                p
            });
            Arc::new(Snapshot { procs: procs.collect(), totals: snap.totals.clone(), cost_ms: snap.cost_ms, ncpu: snap.ncpu, parts: snap.parts.clone() })
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
        if self.palette.is_none() && self.hist.cpu.0.len() == 2 {
            if std::env::var_os("VIGIE_GAME").is_some() {
                self.game_dialog = true;
            }
            if let Ok(q) = std::env::var("VIGIE_PALETTE") {
                self.palette = Some(Palette2 { query: q, sel: 0, just_opened: true });
            }
        }
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
        let matches = |p: &Proc| -> bool {
            if q.is_empty() {
                return true;
            }
            if let Some(port) = q.strip_prefix(':') {
                return p.ports.iter().any(|x| x.port.to_string().starts_with(port));
            }
            q.split_whitespace().all(|w| {
                p.name.to_lowercase().contains(w)
                    || p.key.pid.to_string() == w
                    || p.window.as_deref().map(|t| t.to_lowercase().contains(w)).unwrap_or(false)
                    || p.path.as_deref().and_then(|pa| self.icons.get(pa)).and_then(|e| e.desc.as_deref()).map(|d| d.to_lowercase().contains(w)).unwrap_or(false)
                    || (w.len() >= 3 && p.cmd.as_deref().map(|c| c.to_lowercase().contains(w)).unwrap_or(false))
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
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
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
                for k in &keys {
                    if let Some(e) = self.check_alive(*k).and_then(|_| sys::suspend(k.pid, on).err()) {
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
                    if let Some(e) = self.check_alive(*k).and_then(|_| sys::set_priority(k.pid, p).err()) {
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
                let (n, freed, errs) = self.clean(&ghosts);
                if errs.is_empty() {
                    self.toast(format!("Nettoyage : {n} fantôme(s) chassé(s) · ~{} libérés", human_bytes(freed)), false);
                } else {
                    self.toast(format!("{n} chassé(s), {} échec(s) : {}", errs.len(), errs[0]), true);
                }
                self.sample_now();
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

    /// Le PID désigne-t-il toujours le même processus ? (protège contre la réutilisation des PID)
    fn check_alive(&self, k: Key) -> Option<()> {
        self.proc(k).map(|_| ())
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
            match sys::kill(k.pid) {
                Ok(()) => {
                    ok += 1;
                    freed += mem;
                }
                Err(e) => errs.push(format!("{} : {e}", p.name)),
            }
        }
        (ok, errs, freed)
    }

    pub fn clean(&mut self, ghosts: &[Ghost]) -> (usize, u64, Vec<String>) {
        let (mut n, mut freed, mut errs) = (0, 0u64, Vec::new());
        let mut recs = Vec::new();
        for g in ghosts {
            let res: Result<u64, String> = match &g.fix {
                Fix::KillTree => {
                    let (ok, e, f) = self.kill_keys_quiet(&[g.key], true);
                    errs.extend(e);
                    if ok > 0 { Ok(f) } else { Err(String::new()) }
                }
                Fix::WslShutdown => sys::run_hidden("wsl.exe", &["--shutdown"]).map(|_| g.ram).map_err(|e| format!("wsl --shutdown : {e}")),
                Fix::DockerStop(id) => crate::docker::stop(id).map(|_| g.ram).map_err(|e| format!("docker stop : {e}")),
                Fix::DockerQuit => {
                    let keys: Vec<Key> = self
                        .snap
                        .as_ref()
                        .map(|s| s.procs.iter().filter(|p| p.name.eq_ignore_ascii_case("Docker Desktop.exe") || p.name.eq_ignore_ascii_case("com.docker.backend.exe")).map(|p| p.key).collect())
                        .unwrap_or_default();
                    let (_, e, f) = self.kill_keys_quiet(&keys, true);
                    errs.extend(e);
                    let _ = sys::run_hidden("wsl.exe", &["--terminate", "docker-desktop"]);
                    Ok(f.max(g.ram))
                }
            };
            match res {
                Ok(f) => {
                    n += 1;
                    freed += f;
                    recs.push(crate::journal::Rec { t: crate::journal::now(), what: "chassé".into(), name: g.name.clone(), bytes: f });
                }
                Err(e) if !e.is_empty() => errs.push(e),
                Err(_) => {}
            }
        }
        self.log(recs);
        (n, freed, errs)
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
        let (mut n, mut freed, mut errs) = (0, 0u64, Vec::new());
        let mut by_name: HashMap<String, u64> = HashMap::new();
        for &k in keys {
            let Some(p) = self.proc(k) else { continue };
            let name = p.name.to_string();
            match sys::trim(k.pid) {
                Ok(f) => {
                    n += 1;
                    freed += f;
                    *by_name.entry(name).or_default() += f;
                }
                Err(e) => errs.push(format!("{name} : {e}")),
            }
        }
        let t = crate::journal::now();
        let recs = by_name.into_iter().filter(|(_, b)| *b > 1 << 20).map(|(name, bytes)| crate::journal::Rec { t, what: "veille".into(), name, bytes }).collect();
        self.log(recs);
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

    // ---------- Mode avant de jouer ----------
    pub fn safe_ghosts(&self) -> Vec<Ghost> {
        self.ghosts.iter().filter(|g| g.safe).cloned().collect()
    }

    pub fn run_game_mode(&mut self) {
        let mut parts = Vec::new();
        let mut errs = Vec::new();
        if self.s.game_kill {
            let g = self.safe_ghosts();
            let (n, freed, e) = self.clean(&g);
            self.game_freed += freed;
            if n > 0 {
                parts.push(format!("{n} fantôme(s), {}", human_bytes(freed)));
            }
            errs.extend(e);
        }
        if self.s.game_trim {
            let keys = self.trim_candidates();
            let (n, freed, _) = self.trim_keys(&keys);
            // retenus pour le réveil de --apres-jeu / « Quitter le mode jeu »
            crate::cli::noter_veille(&keys);
            self.game_freed += freed;
            if n > 0 && freed > 0 {
                parts.push(format!("{n} app(s) en veille, {}", human_bytes(freed)));
            }
        }
        if self.s.game_purge && self.elevated {
            let before = sys::standby_bytes().unwrap_or(0);
            match sys::purge_standby() {
                Ok(()) => {
                    parts.push(format!("cache vidé ({})", human_bytes(before)));
                    self.log(vec![crate::journal::Rec { t: crate::journal::now(), what: "cache".into(), name: "mémoire en veille".into(), bytes: before }]);
                }
                Err(e) => errs.push(e),
            }
        }
        if self.s.game_power {
            match enter_power() {
                Ok(Some(prev)) => {
                    if self.s.prev_power.is_none() {
                        self.s.prev_power = Some(prev);
                    }
                    parts.push("plan Performances élevées".into());
                }
                Ok(None) => {}
                Err(e) => errs.push(format!("plan d’alimentation : {e}")),
            }
        }
        self.game_active = true;
        self.s.save();
        let msg = if parts.is_empty() { "Mode jeu actif — rien à nettoyer".to_string() } else { format!("Mode jeu : {}", parts.join(" · ")) };
        if errs.is_empty() {
            self.toast(msg, false);
        } else {
            self.toast(format!("{msg} — {}", errs[0]), true);
        }
        self.sample_now();
    }

    pub fn exit_game_mode(&mut self) {
        let (msg, err, _) = finish_game(&mut self.s);
        self.toast(msg, err);
        self.game_active = false;
    }
}

/// Fin du mode jeu : restaure le plan d'alimentation et réveille les apps mises en veille.
/// Partagé par l'interface et `--apres-jeu` (qui n'ouvre ni fenêtre ni thread de mesure).
pub fn finish_game(s: &mut Settings) -> (String, bool, usize) {
    let mut parts = vec!["Mode jeu terminé".to_string()];
    let mut err = false;
    if let Some(prev) = s.prev_power.take() {
        match sys::set_power_scheme(&prev) {
            Ok(()) => parts.push(format!("plan « {} » restauré", sys::power_scheme_name(&prev))),
            Err(e) => {
                parts.push(format!("plan d’alimentation : {e}"));
                err = true;
            }
        }
    }
    let (n, b) = crate::cli::reveiller();
    if n > 0 {
        parts.push(format!("{n} app(s) réveillée(s) ({} préchargés)", human_bytes(b)));
    }
    s.save();
    (parts.join(" · "), err, n)
}

/// Passe en Performances élevées ; renvoie le plan précédent (à restaurer).
pub fn enter_power() -> Result<Option<String>, String> {
    let prev = sys::active_power_scheme();
    // déjà en Performances élevées / optimales (Ultimate, y compris ses copies) : ne surtout pas rétrograder
    if prev.as_deref() == Some(sys::HIGH_PERF) || sys::active_power_is_max() {
        return Ok(None);
    }
    if sys::set_power_scheme(sys::HIGH_PERF).is_err() {
        // plan masqué sur certains PC : on le recrée depuis le modèle Windows
        let _ = sys::run_hidden("powercfg.exe", &["-duplicatescheme", sys::HIGH_PERF, sys::HIGH_PERF]);
        sys::set_power_scheme(sys::HIGH_PERF)?;
    }
    Ok(prev)
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
