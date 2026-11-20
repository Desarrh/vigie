//! Thread d'échantillonnage : une mesure par intervalle, envoyée à l'interface qui ne redessine
//! qu'à réception. Rien ne tourne entre deux mesures (attente bloquante sur le canal de commandes).
use crate::pdh::Pdh;
use crate::sys::{self, Port, Raw};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// Identité stable d'un processus (un PID peut être réutilisé).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Key {
    pub pid: u32,
    pub ctime: i64,
}

#[derive(Clone)]
pub struct Proc {
    pub key: Key,
    pub ppid: u32,
    /// parent encore vivant (même PID ET créé avant l'enfant)
    pub parent: Option<Key>,
    pub name: Arc<str>,
    pub path: Option<Arc<str>>,
    pub cmd: Option<Arc<str>>,
    pub session: u32,
    pub threads: u32,
    pub handles: u32,
    pub base_prio: i32,
    pub suspended: bool,
    pub cpu: f32,
    pub mem: u64,
    pub commit: u64,
    pub ws: u64,
    pub disk: f64,
    pub gpu: f32,
    pub gpu_mem: u64,
    pub window: Option<Arc<str>>,
    pub ports: Arc<[Port]>,
    /// secondes depuis la dernière activité CPU notable
    pub idle_secs: f32,
    /// âge du processus (s)
    pub age_secs: f32,
    /// déjà là à l'ouverture de Vigie et jamais vu actif depuis : `idle_secs` est une estimation
    pub idle_min: bool,
}

#[derive(Clone, Default)]
pub struct Totals {
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub commit: u64,
    pub commit_limit: u64,
    pub standby: Option<u64>,
    pub gpu: f32,
    pub gpu_mem: u64,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
    pub threads: u32,
    pub handles: u32,
}

pub struct Snapshot {
    pub procs: Vec<Proc>,
    pub totals: Totals,
    /// coût de la mesure (ms), affiché dans la barre d'état
    pub cost_ms: f32,
    pub ncpu: u32,
    /// détail du coût de la mesure
    pub parts: String,
}

/// Intervalle de mesure quand la fenêtre est réduite.
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(10);

pub enum Cmd {
    Interval(Duration),
    Gpu(bool),
    /// mesurer tout de suite (après un kill, par exemple)
    Now,
    /// fenêtre réduite : ralentir
    Background(bool),
}

struct Prev {
    cpu_time: i64,
    io: u64,
    last_active: Instant,
    /// jamais vu actif depuis le début de l'observation
    never: bool,
}

struct Cached {
    path: Option<Arc<str>>,
    cmd: Option<Arc<str>>,
}

pub fn spawn(interval: Duration, gpu: bool, out: impl Fn(Arc<Snapshot>) + Send + 'static) -> Sender<Cmd> {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    std::thread::Builder::new()
        .name("echantillonnage".into())
        .spawn(move || run(rx, interval, gpu, out))
        .expect("thread d'échantillonnage");
    tx
}

fn run(rx: Receiver<Cmd>, mut interval: Duration, mut gpu: bool, out: impl Fn(Arc<Snapshot>)) {
    let ncpu = sys::cpu_count();
    let mut buf = Vec::new();
    let mut pdh = Pdh::new(gpu);
    let mut prev: HashMap<Key, Prev> = HashMap::new();
    let mut cache: HashMap<Key, Cached> = HashMap::new();
    let mut last_t = Instant::now();
    let mut last_sys = sys::system_times();
    let mut background = false;
    let mut tick: u64 = 0;
    let mut ports: HashMap<u32, Vec<Port>> = HashMap::new();
    let mut ports_arc: HashMap<u32, Arc<[Port]>> = HashMap::new();
    let empty_ports: Arc<[Port]> = Arc::from(Vec::new());
    let mut names: HashMap<String, Arc<str>> = HashMap::new();
    let mut last_pdh = crate::pdh::Sample::default();
    loop {
        let t0 = Instant::now();
        let raws = sys::query_processes(&mut buf);
        let t_procs = t0.elapsed();
        let now = Instant::now();
        let dt = now.duration_since(last_t).as_secs_f64().max(0.05);
        last_t = now;
        let sys_t = sys::system_times();
        let (di, dtot) = (sys_t.0.saturating_sub(last_sys.0), sys_t.1.saturating_sub(last_sys.1));
        last_sys = sys_t;
        let cpu_total = if dtot > 0 { (1.0 - di as f64 / dtot as f64).clamp(0.0, 1.0) as f32 * 100.0 } else { 0.0 };

        // Fenêtres et ports : chaque mesure au premier plan, une sur trois en arrière-plan.
        let wins = sys::windows();
        let t_wins = t0.elapsed();
        if !background || tick % 3 == 0 {
            ports = sys::ports();
            ports_arc = ports.iter().map(|(k, v)| (*k, Arc::from(v.clone()))).collect();
        }
        let t_ports = t0.elapsed();
        // En arrière-plan (fenêtre réduite) : pas de compteurs PDH (GPU, disque, réseau ≈ 2,6 ms),
        // on garde les dernières valeurs. Restent la lecture noyau (pics de RAM des jeux, chasseur)
        // et les fenêtres ; les ports une mesure sur trois.
        let pdhs = if background {
            last_pdh.clone()
        } else {
            let s = pdh.as_mut().map(|p| p.sample()).unwrap_or_default();
            last_pdh = s.clone();
            s
        };
        let t_pdh = t0.elapsed();

        let ctime_of: HashMap<u32, i64> = raws.iter().map(|r| (r.pid, r.create_time)).collect();
        let mut procs = Vec::with_capacity(raws.len());
        let mut seen: HashMap<Key, Prev> = HashMap::with_capacity(raws.len());
        let (mut threads, mut handles, mut gpu_mem_total) = (0u32, 0u32, 0u64);
        let now_ft = filetime_now();
        for r in &raws {
            let key = Key { pid: r.pid, ctime: r.create_time };
            let io = r.io_read + r.io_write + r.io_other;
            let (cpu, disk, last_active, never) = match prev.get(&key) {
                Some(p) => {
                    let dc = (r.cpu_time - p.cpu_time).max(0) as f64 / 1e7;
                    let cpu = (dc / dt / ncpu as f64 * 100.0) as f32;
                    let disk = io.saturating_sub(p.io) as f64 / dt;
                    // « actif » = au moins 1 % d'un cœur pendant la mesure
                    let active = dc / dt > 0.01 && r.pid != 0;
                    if active { (cpu, disk, now, false) } else { (cpu, disk, p.last_active, p.never) }
                }
                // déjà là à l'ouverture de Vigie : durée d'inactivité inconnue avant
                // Si son CPU moyen sur toute sa vie est quasi nul, on estime qu'il dort depuis son lancement.
                None if tick == 0 => {
                    let age = ((filetime_now() - r.create_time).max(0) as f64 / 1e7).max(1.0);
                    let avg = r.cpu_time as f64 / 1e7 / age;
                    let since = if avg < 0.003 { Duration::from_secs_f64(age.min(86_400.0)) } else { Duration::ZERO };
                    (0.0, 0.0, now.checked_sub(since).unwrap_or(now), true)
                }
                None => (0.0, 0.0, now, false),
            };
            seen.insert(key, Prev { cpu_time: r.cpu_time, io, last_active, never });
            let c = cache.entry(key).or_insert_with(|| fetch_cached(r));
            let parent = ctime_of
                .get(&r.ppid)
                .filter(|&&pc| r.ppid != r.pid && r.ppid != 0 && pc <= r.create_time)
                .map(|&pc| Key { pid: r.ppid, ctime: pc });
            threads += r.threads;
            handles += r.handles;
            let gm = pdhs.gpu_mem.get(&r.pid).copied().unwrap_or(0);
            gpu_mem_total += gm;
            let name = names.entry(r.name.clone()).or_insert_with(|| Arc::from(r.name.as_str())).clone();
            procs.push(Proc {
                key,
                ppid: r.ppid,
                parent,
                name,
                path: c.path.clone(),
                cmd: c.cmd.clone(),
                session: r.session,
                threads: r.threads,
                handles: r.handles,
                base_prio: r.base_prio,
                suspended: r.suspended,
                cpu: if r.pid == 0 { 0.0 } else { cpu },
                mem: r.private_ws,
                commit: r.commit,
                ws: r.ws,
                disk,
                gpu: pdhs.gpu.get(&r.pid).copied().unwrap_or(0.0),
                gpu_mem: gm,
                window: wins.get(&r.pid).map(|s| Arc::from(s.as_str())),
                ports: ports_arc.get(&r.pid).cloned().unwrap_or_else(|| empty_ports.clone()),
                idle_secs: now.duration_since(last_active).as_secs_f32(),
                age_secs: ((now_ft - r.create_time).max(0) as f64 / 1e7) as f32,
                idle_min: never,
            });
        }
        cache.retain(|k, _| seen.contains_key(k));
        prev = seen;
        let m = sys::memory();
        let standby = if tick % 5 == 0 || !background { sys::standby_bytes() } else { None };
        let totals = Totals {
            cpu: cpu_total,
            mem_used: m.total - m.avail,
            mem_total: m.total,
            commit: m.commit_total,
            commit_limit: m.commit_limit,
            standby,
            gpu: pdhs.gpu_total,
            gpu_mem: if pdhs.gpu_mem_total > 0 { pdhs.gpu_mem_total } else { gpu_mem_total },
            disk_read: pdhs.disk_read,
            disk_write: pdhs.disk_write,
            net_rx: pdhs.net_rx,
            net_tx: pdhs.net_tx,
            threads,
            handles,
        };
        let cost_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        let parts = format!(
            "processus {:.1} ms · fenêtres {:.1} · ports {:.1} · compteurs PDH {:.1} · reste {:.1}",
            ms(t_procs),
            ms(t_wins - t_procs),
            ms(t_ports - t_wins),
            ms(t_pdh - t_ports),
            cost_ms - ms(t_pdh)
        );
        out(Arc::new(Snapshot { procs, totals, cost_ms, ncpu, parts }));
        tick += 1;
        let _ = &ports;

        // Attente jusqu'à la prochaine mesure — ou une commande.
        // réduite : une mesure toutes les 10 s au plus (CPU ≈ 0 au repos, règle 1)
        let wait = if background { interval.max(BACKGROUND_INTERVAL) } else { interval };
        let deadline = Instant::now() + wait.saturating_sub(t0.elapsed());
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(Cmd::Interval(d)) => interval = d,
                Ok(Cmd::Gpu(on)) => {
                    if on != gpu {
                        gpu = on;
                        pdh = Pdh::new(gpu);
                    }
                }
                Ok(Cmd::Now) => {
                    // laisse le temps au noyau de retirer le processus tué
                    std::thread::sleep(Duration::from_millis(120));
                    break;
                }
                Ok(Cmd::Background(b)) => {
                    let was = background;
                    background = b;
                    if was && !b {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn fetch_cached(r: &Raw) -> Cached {
    if r.pid <= 4 {
        return Cached { path: None, cmd: None };
    }
    Cached { path: sys::process_path(r.pid).map(Arc::from), cmd: sys::process_cmdline(r.pid).map(Arc::from) }
}

fn filetime_now() -> i64 {
    let mut f = windows_sys::Win32::Foundation::FILETIME::default();
    unsafe { windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime(&mut f) };
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}
