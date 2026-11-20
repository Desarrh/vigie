//! Interface en ligne de commande pour les autres applis (le launcher Ultimate surtout).
//! Sans fenêtre et sans egui : un seul instantané noyau, réponse sur la sortie standard.
//!
//! - `--etat-dev` : serveurs de dev, Docker et WSL en cours, une ligne JSON (format défini avec Ultimate).
//! - `--couper-dev [--serveurs] [--docker] [--wsl] [--json]` : les arrête (tout par défaut), résumé d'une ligne.
//! - `--apres-jeu` : restaure le plan d'alimentation et réveille les apps mises en veille par `--avant-jeu`.
//!
//! Jamais touchés : un arbre qui a une fenêtre visible (le jeu, un éditeur…), les ancêtres de l'appelant
//! (Ultimate, même lancé par `npm run tauri dev`), Claude Code, les processus protégés du chasseur.
use crate::app::human_bytes;
use crate::hunter;
use crate::sampler::Key;
use crate::sys::{self, Port, Raw};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Un élément coupable, au format attendu par Ultimate.
#[derive(Serialize)]
pub struct Item {
    /// "node" | "python" | "docker" | "wsl" | "autre"
    pub kind: &'static str,
    /// ex. « npm run dev (C:\Dev\Launcher) »
    pub label: String,
    pub pids: Vec<u32>,
    pub ram_mo: u64,
    /// champs en plus (tolérés par Ultimate)
    pub ports: Vec<u16>,
    pub projet: Option<String>,
}

#[derive(Serialize)]
pub struct Etat {
    pub v: u32,
    pub items: Vec<Item>,
    pub total_mo: u64,
    pub ms: f32,
}

const DOCKER: &[&str] = &["docker desktop.exe", "com.docker.backend.exe", "com.docker.build.exe", "com.docker.dev-envs.exe", "docker-sandbox.exe"];
const JAMAIS: &[&str] = &["ultimate.exe", "vigie.exe"];

struct Vue {
    raws: Vec<Raw>,
    keys: HashMap<u32, Key>,
    parent: HashMap<Key, Key>,
    kids: HashMap<Key, Vec<Key>>,
    ports: HashMap<u32, Vec<Port>>,
    fenetres: HashSet<u32>,
    /// Vigie et tous ses ancêtres (dont l'appelant, Ultimate)
    intouchables: HashSet<Key>,
}

impl Vue {
    fn lire() -> Vue {
        let mut buf = Vec::new();
        let raws = sys::query_processes(&mut buf);
        let keys: HashMap<u32, Key> = raws.iter().map(|r| (r.pid, Key { pid: r.pid, ctime: r.create_time })).collect();
        let mut parent = HashMap::new();
        let mut kids: HashMap<Key, Vec<Key>> = HashMap::new();
        for r in &raws {
            if let Some(pk) = keys.get(&r.ppid).filter(|pk| r.ppid != r.pid && r.ppid != 0 && pk.ctime <= r.create_time) {
                let k = keys[&r.pid];
                parent.insert(k, *pk);
                kids.entry(*pk).or_default().push(k);
            }
        }
        let mut intouchables = HashSet::new();
        let mut cur = keys.get(&std::process::id()).copied();
        while let Some(k) = cur {
            if !intouchables.insert(k) {
                break;
            }
            cur = parent.get(&k).copied();
        }
        let fenetres = sys::windows().into_keys().collect();
        Vue { raws, keys, parent, kids, ports: sys::ports(), fenetres, intouchables }
    }
    fn raw(&self, k: Key) -> Option<&Raw> {
        self.raws.iter().find(|r| r.pid == k.pid && r.create_time == k.ctime)
    }
    fn arbre(&self, k: Key) -> Vec<Key> {
        let mut out = vec![k];
        let mut i = 0;
        while i < out.len() && out.len() < 5000 {
            if let Some(c) = self.kids.get(&out[i]) {
                out.extend(c.iter().copied());
            }
            i += 1;
        }
        out
    }
    fn ram(&self, ks: &[Key]) -> u64 {
        ks.iter().filter_map(|k| self.raw(*k)).map(|r| r.private_ws).sum()
    }
    /// Arbre qu'on n'a pas le droit de couper (fenêtre visible, appelant, Ultimate, protégés…).
    fn interdit(&self, ks: &[Key]) -> bool {
        ks.iter().any(|k| {
            let Some(r) = self.raw(*k) else { return false };
            let n = lower(r);
            let console = n == "conhost.exe" || n == "openconsole.exe";
            self.intouchables.contains(k) || self.fenetres.contains(&k.pid) || JAMAIS.contains(&n.as_str()) || (!console && hunter::is_protected_name(&n))
        })
    }
}

fn lower(r: &Raw) -> String {
    r.name.to_ascii_lowercase()
}

fn mo(b: u64) -> u64 {
    (b + (1 << 19)) >> 20
}

/// `C:\Dev\x\…` dans la ligne de commande → `C:\Dev\x`.
fn projet(cmd: &str) -> Option<String> {
    let low = cmd.to_lowercase();
    let i = low.find(r"c:\dev\")?;
    let rest = &cmd[i + 7..];
    let name: String = rest.chars().take_while(|c| !matches!(c, '\\' | '/' | '"' | ' ')).collect();
    (!name.is_empty()).then(|| format!(r"C:\Dev\{name}"))
}

fn kind_of(name: &str) -> &'static str {
    match name.trim_end_matches(".exe") {
        "node" | "bun" | "deno" | "npm" | "pnpm" | "yarn" | "vite" | "esbuild" | "turbo" | "electron" => "node",
        "python" | "python3" | "pythonw" | "py" | "uvicorn" | "gunicorn" | "flask" => "python",
        _ => "autre",
    }
}

/// Libellé lisible : « npm run dev », « vite », « node server.js », « cargo watch »… + (projet).
fn label(name: &str, cmd: Option<&str>) -> String {
    let base = name.trim_end_matches(".exe").trim_end_matches(".EXE");
    let low = cmd.unwrap_or("").to_lowercase();
    let args = hunter::split_args(cmd.unwrap_or(""));
    let mut l = if let Some(p) = low.find(" run ").filter(|_| low.contains("npm") || low.contains("pnpm") || low.contains("yarn")) {
        let mgr = if low.contains("pnpm") {
            "pnpm"
        } else if low.contains("yarn") {
            "yarn"
        } else {
            "npm"
        };
        let t = low[p + 5..].split_whitespace().next().unwrap_or("").to_string();
        format!("{mgr} run {t}")
    } else if low.contains("vite") {
        "vite".into()
    } else if low.contains("next") && low.contains(" dev") {
        "next dev".into()
    } else if base.eq_ignore_ascii_case("cargo") || base.eq_ignore_ascii_case("dotnet") {
        args.iter().skip(1).take(2).fold(base.to_string(), |a, b| format!("{a} {}", b.trim_matches('"')))
    } else if let Some(sc) = args.get(1).map(|a| a.trim_matches('"').to_string()).filter(|a| !a.starts_with('-')) {
        let f = std::path::Path::new(&sc).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(sc);
        format!("{base} {f}")
    } else {
        base.to_string()
    };
    if let Some(p) = cmd.and_then(projet) {
        l = format!("{l} ({p})");
    }
    l
}

/// Serveurs de dev : outil de dev de la session de l'utilisateur qui écoute sur un port TCP,
/// regroupé par racine (npm → node → esbuild).
fn serveurs(v: &Vue, session: u32) -> Vec<(Key, Vec<Key>)> {
    let mut vus: HashSet<Key> = HashSet::new();
    let mut out = Vec::new();
    for r in &v.raws {
        let n = lower(r);
        if r.session != session || !hunter::is_dev_tool(&n) {
            continue;
        }
        if !v.ports.get(&r.pid).map(|ps| ps.iter().any(|p| p.tcp)).unwrap_or(false) {
            continue;
        }
        let mut root = v.keys[&r.pid];
        while let Some(par) = v.parent.get(&root) {
            match v.raw(*par) {
                Some(pr) if hunter::is_dev_tool(&lower(pr)) => root = *par,
                _ => break,
            }
        }
        if !vus.insert(root) {
            continue;
        }
        let ks = v.arbre(root);
        if v.interdit(&ks) {
            continue;
        }
        let ia = [r.pid, root.pid].iter().any(|&pid| {
            let c = sys::process_cmdline(pid).unwrap_or_default().to_ascii_lowercase();
            c.contains("@anthropic-ai") || c.contains("claude-code")
        });
        if !ia {
            out.push((root, ks));
        }
    }
    out
}

fn docker_keys(v: &Vue) -> Vec<Key> {
    let mut ks: Vec<Key> = Vec::new();
    for r in v.raws.iter().filter(|r| DOCKER.contains(&lower(r).as_str())) {
        for k in v.arbre(v.keys[&r.pid]) {
            if !ks.contains(&k) {
                ks.push(k);
            }
        }
    }
    ks
}

fn wsl_raws(v: &Vue) -> Vec<&Raw> {
    v.raws.iter().filter(|r| lower(r).starts_with("vmmemwsl")).collect()
}

pub fn etat() -> Etat {
    let t0 = std::time::Instant::now();
    let v = Vue::lire();
    let mut items = Vec::new();
    for (root, ks) in serveurs(&v, sys::current_session()) {
        let cmd = sys::process_cmdline(root.pid);
        let name = v.raw(root).map(|r| r.name.clone()).unwrap_or_default();
        let mut ports: Vec<u16> = ks.iter().flat_map(|k| v.ports.get(&k.pid).into_iter().flatten().filter(|p| p.tcp).map(|p| p.port)).collect();
        ports.sort();
        ports.dedup();
        items.push(Item {
            kind: kind_of(&name.to_ascii_lowercase()),
            label: label(&name, cmd.as_deref()),
            pids: ks.iter().map(|k| k.pid).collect(),
            ram_mo: mo(v.ram(&ks)),
            ports,
            projet: cmd.as_deref().and_then(projet),
        });
    }
    let dk = docker_keys(&v);
    if !dk.is_empty() {
        items.push(Item { kind: "docker", label: "Docker Desktop".into(), pids: dk.iter().map(|k| k.pid).collect(), ram_mo: mo(v.ram(&dk)), ports: vec![], projet: None });
    }
    let w = wsl_raws(&v);
    if !w.is_empty() {
        items.push(Item {
            kind: "wsl",
            label: "WSL (vmmemWSL)".into(),
            pids: w.iter().map(|r| r.pid).collect(),
            ram_mo: mo(w.iter().map(|r| r.ws.max(r.private_ws)).sum()),
            ports: vec![],
            projet: None,
        });
    }
    let total_mo = items.iter().map(|i| i.ram_mo).sum();
    Etat { v: 1, items, total_mo, ms: t0.elapsed().as_secs_f32() * 1000.0 }
}

#[derive(Serialize, Default)]
pub struct Coupe {
    pub serveurs: usize,
    pub docker: bool,
    pub wsl: bool,
    /// estimation de la RAM libérée (octets)
    pub libere: u64,
    /// actions sautées (admin requis…)
    pub sautes: Vec<String>,
    pub erreurs: Vec<String>,
}

impl Coupe {
    /// « 3 serveurs coupés, WSL arrêté, 5,9 Go libérés »
    pub fn resume(&self) -> String {
        let mut p = Vec::new();
        if self.serveurs > 0 {
            let s = if self.serveurs > 1 { "s" } else { "" };
            p.push(format!("{} serveur{s} coupé{s}", self.serveurs));
        }
        if self.docker {
            p.push("Docker arrêté".into());
        }
        if self.wsl {
            p.push("WSL arrêté".into());
        }
        let mut s = if p.is_empty() { "Rien à couper".to_string() } else { format!("{}, {} libérés", p.join(", "), human_bytes(self.libere)) };
        if !self.sautes.is_empty() {
            s += &format!(" · sauté : {}", self.sautes.join(", "));
        }
        if !self.erreurs.is_empty() {
            s += &format!(" · {} échec(s) : {}", self.erreurs.len(), self.erreurs[0]);
        }
        s
    }
}

fn tuer_arbre(v: &Vue, ks: &[Key], c: &mut Coupe) -> (usize, u64) {
    let (mut n, mut b) = (0, 0u64);
    // enfants d'abord
    for k in ks.iter().rev() {
        let Some(r) = v.raw(*k) else { continue };
        match sys::kill(k.pid) {
            Ok(()) => {
                n += 1;
                b += r.private_ws;
            }
            Err(e) if lower(r) != "conhost.exe" => {
                // accès refusé = processus élevé : on saute, sans demander l'admin
                if e.contains("admin") {
                    c.sautes.push(format!("{} (admin requis)", r.name));
                } else {
                    c.erreurs.push(format!("{} : {e}", r.name));
                }
            }
            Err(_) => {}
        }
    }
    (n, b)
}

pub fn couper(serv: bool, dock: bool, wsl: bool) -> Coupe {
    let v = Vue::lire();
    let mut c = Coupe::default();
    let mut recs = Vec::new();
    let now = crate::journal::now();
    if serv {
        for (root, ks) in serveurs(&v, sys::current_session()) {
            let name = v.raw(root).map(|r| r.name.clone()).unwrap_or_default();
            let (n, b) = tuer_arbre(&v, &ks, &mut c);
            if n > 0 {
                c.serveurs += 1;
                c.libere += b;
                recs.push(crate::journal::Rec { t: now, what: "coupé".into(), name, bytes: b });
            }
        }
    }
    if dock {
        let ks = docker_keys(&v);
        if !ks.is_empty() {
            let b = v.ram(&ks);
            // proprement d'abord (`docker desktop stop`, Docker Desktop 4.37+), sinon on arrête ses processus
            if sys::run_hidden("docker", &["desktop", "stop"]).is_err() {
                tuer_arbre(&v, &ks, &mut c);
            }
            let _ = sys::run_hidden("wsl.exe", &["--terminate", "docker-desktop"]);
            c.docker = true;
            c.libere += b;
            recs.push(crate::journal::Rec { t: now, what: "coupé".into(), name: "Docker Desktop".into(), bytes: b });
        }
    }
    if wsl {
        let vm: u64 = wsl_raws(&v).iter().map(|r| r.ws.max(r.private_ws)).sum();
        if vm > 0 {
            match sys::run_hidden("wsl.exe", &["--shutdown"]) {
                Ok(_) => {
                    c.wsl = true;
                    c.libere += vm;
                    recs.push(crate::journal::Rec { t: now, what: "coupé".into(), name: "WSL".into(), bytes: vm });
                }
                Err(e) => c.erreurs.push(format!("wsl --shutdown : {e}")),
            }
        }
    }
    crate::journal::append(&recs);
    c
}

#[cfg(test)]
mod tests {
    #[test]
    fn libelles() {
        let l = super::label("node.exe", Some(r#""C:\Program Files\nodejs\node.exe" "C:\Users\x\AppData\Roaming\npm\node_modules\npm\bin\npm-cli.js" run dev"#));
        assert_eq!(l, "npm run dev");
        let l = super::label("node.exe", Some(r#""C:\Program Files\nodejs\node.exe" C:\Dev\Launcher\node_modules\vite\bin\vite.js"#));
        assert_eq!(l, r"vite (C:\Dev\Launcher)");
        let l = super::label("node.exe", Some(r"node C:\Dev\vigie\target\tmp\serveur.js"));
        assert_eq!(l, r"node serveur.js (C:\Dev\vigie)");
        assert_eq!(super::label("cargo.exe", Some("cargo watch -x run")), "cargo watch -x");
    }
}

// ---------- Apps mises en veille par --avant-jeu ----------
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Endormi {
    pub pid: u32,
    pub ctime: i64,
}

fn fichier_veille() -> Option<std::path::PathBuf> {
    crate::app::Settings::dir().map(|d| d.join("veille.json"))
}

pub fn noter_veille(keys: &[Key]) {
    let Some(p) = fichier_veille() else { return };
    let v: Vec<Endormi> = keys.iter().map(|k| Endormi { pid: k.pid, ctime: k.ctime }).collect();
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = std::fs::write(p, serde_json::to_vec(&v).unwrap_or_default());
}

/// Réveil : précharge la mémoire des apps mises en veille (lectures disque en arrière-plan par Windows),
/// pour qu'elles ne rament pas au premier clic après le jeu. Renvoie (processus réveillés, octets demandés).
pub fn reveiller() -> (usize, u64) {
    let Some(p) = fichier_veille() else { return (0, 0) };
    let Ok(b) = std::fs::read(&p) else { return (0, 0) };
    let _ = std::fs::remove_file(&p);
    let list: Vec<Endormi> = serde_json::from_slice(&b).unwrap_or_default();
    let mut buf = Vec::new();
    let vivants: HashSet<(u32, i64)> = sys::query_processes(&mut buf).into_iter().map(|r| (r.pid, r.create_time)).collect();
    let (mut n, mut total) = (0, 0u64);
    for e in list.into_iter().filter(|e| vivants.contains(&(e.pid, e.ctime))) {
        if let Ok(b) = sys::prefetch(e.pid) {
            n += 1;
            total += b;
        }
    }
    (n, total)
}

/// Résultat JSON de --avant-jeu / --apres-jeu.
#[derive(Serialize)]
pub struct Jeu {
    pub message: String,
    pub libere: u64,
    pub reveilles: usize,
}
