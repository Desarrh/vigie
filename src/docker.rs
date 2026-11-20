//! Conteneurs Docker : interrogés par la CLI `docker`, seulement quand Docker Desktop tourne,
//! sur un thread à part, toutes les 20 s (la CLI met 0,2–1 s à répondre).
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub mem: u64,
    pub cpu: f32,
    /// secondes sans activité CPU notable (depuis que Vigie observe)
    pub idle_secs: f32,
    /// déjà là à l'ouverture de Vigie : durée d'inactivité inconnue avant
    pub idle_min: bool,
}

#[derive(Default)]
pub struct State {
    pub containers: Vec<Container>,
    pub error: Option<String>,
    pub updated: Option<Instant>,
}

pub type Shared = Arc<Mutex<State>>;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Ps {
    #[serde(rename = "ID")]
    id: String,
    names: String,
    image: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Stats {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "CPUPerc")]
    cpu_perc: String,
    mem_usage: String,
}

/// « 123.4MiB / 7.6GiB » → octets de la partie gauche.
pub fn parse_mem(s: &str) -> u64 {
    let left = s.split('/').next().unwrap_or("").trim();
    let i = left.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(left.len());
    let v: f64 = left[..i].trim().parse().unwrap_or(0.0);
    let mult = match left[i..].trim().to_ascii_lowercase().as_str() {
        "kib" => 1024.0,
        "kb" => 1000.0,
        "mib" => 1024.0 * 1024.0,
        "mb" => 1e6,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "gb" => 1e9,
        _ => 1.0,
    };
    (v * mult) as u64
}

fn docker(args: &[&str]) -> Result<String, String> {
    crate::sys::run_hidden("docker", args)
}

/// Thread de surveillance ; `running` dit s'il faut interroger (Docker Desktop présent).
pub fn spawn(running: Arc<std::sync::atomic::AtomicBool>, repaint: impl Fn() + Send + 'static) -> Shared {
    let shared: Shared = Arc::new(Mutex::new(State::default()));
    let out = shared.clone();
    std::thread::Builder::new()
        .name("docker".into())
        .spawn(move || {
            let mut last_active: HashMap<String, (Instant, bool)> = HashMap::new();
            let mut first = true;
            loop {
                if running.load(std::sync::atomic::Ordering::Relaxed) {
                    let res = (|| -> Result<Vec<Container>, String> {
                        let ps = docker(&["ps", "--no-trunc", "--format", "{{json .}}"])?;
                        let list: Vec<Ps> = ps.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
                        if list.is_empty() {
                            return Ok(Vec::new());
                        }
                        let st = docker(&["stats", "--no-stream", "--no-trunc", "--format", "{{json .}}"])?;
                        let stats: HashMap<String, Stats> = st.lines().filter_map(|l| serde_json::from_str::<Stats>(l).ok()).map(|s| (s.id.clone(), s)).collect();
                        let now = Instant::now();
                        Ok(list
                            .into_iter()
                            .map(|p| {
                                let s = stats.get(&p.id);
                                let cpu: f32 = s.map(|s| s.cpu_perc.trim_end_matches('%').parse().unwrap_or(0.0)).unwrap_or(0.0);
                                let mem = s.map(|s| parse_mem(&s.mem_usage)).unwrap_or(0);
                                let e = last_active.entry(p.id.clone()).or_insert((now, first));
                                if cpu > 0.5 {
                                    *e = (now, false);
                                }
                                Container {
                                    id: p.id,
                                    name: p.names,
                                    image: p.image,
                                    mem,
                                    cpu,
                                    idle_secs: now.duration_since(e.0).as_secs_f32(),
                                    idle_min: e.1,
                                }
                            })
                            .collect())
                    })();
                    first = false;
                    if let Ok(mut s) = out.lock() {
                        match res {
                            Ok(c) => {
                                s.containers = c;
                                s.error = None;
                            }
                            Err(e) => {
                                s.containers.clear();
                                s.error = Some(e);
                            }
                        }
                        s.updated = Some(Instant::now());
                    }
                    repaint();
                    std::thread::sleep(Duration::from_secs(20));
                } else {
                    if let Ok(mut s) = out.lock() {
                        if s.updated.is_some() {
                            *s = State::default();
                        }
                    }
                    first = true;
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        })
        .ok();
    shared
}

pub fn stop(id: &str) -> Result<(), String> {
    docker(&["stop", id]).map(|_| ())
}

#[cfg(test)]
mod tests {
    #[test]
    fn mem() {
        assert_eq!(super::parse_mem("512MiB / 7.6GiB"), 512 * 1024 * 1024);
        assert_eq!(super::parse_mem("1.5GiB / 7.6GiB"), (1.5 * 1024.0 * 1024.0 * 1024.0) as u64);
    }
}
