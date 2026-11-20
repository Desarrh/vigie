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
    /// ligne de commande d'un PID (réelle, ou fausse dans les tests)
    cmd: Box<dyn Fn(u32) -> Option<String>>,
}

impl Vue {
    fn lire() -> Vue {
        let mut buf = Vec::new();
        let raws = sys::query_processes(&mut buf);
        let fenetres = sys::windows().into_keys().collect();
        Vue::assembler(raws, sys::ports(), fenetres, std::process::id(), Box::new(sys::process_cmdline))
    }

    /// Vue à partir de données fournies (réelles, ou fausses dans les tests).
    fn assembler(raws: Vec<Raw>, ports: HashMap<u32, Vec<Port>>, fenetres: HashSet<u32>, moi: u32, cmd: Box<dyn Fn(u32) -> Option<String>>) -> Vue {
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
        let intouchables = crate::qol::ancestors(keys.get(&moi).copied(), |k| parent.get(&k).copied());
        Vue { raws, keys, parent, kids, ports, fenetres, intouchables, cmd }
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
    let low = cmd.to_ascii_lowercase(); // même longueur que `cmd` : le découpage reste valide
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
    let low = cmd.unwrap_or("").to_ascii_lowercase();
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
        let ia = [r.pid, root.pid].iter().any(|&pid| (v.cmd)(pid).map(|c| hunter::is_ai_cmd(&c)).unwrap_or(false));
        if !ia {
            out.push((root, ks));
        }
    }
    out
}

fn docker_keys(v: &Vue) -> Vec<Key> {
    let mut ks: Vec<Key> = Vec::new();
    for r in v.raws.iter().filter(|r| hunter::is_docker_name(&lower(r))) {
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
        let cmd = (v.cmd)(root.pid);
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
        match sys::kill(k.pid, k.ctime) {
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
    couper_vue(&Vue::lire(), serv, dock, wsl, None)
}

/// `seulement` : ne couper que ces racines de serveurs (tests).
fn couper_vue(v: &Vue, serv: bool, dock: bool, wsl: bool, seulement: Option<&HashSet<u32>>) -> Coupe {
    let mut c = Coupe::default();
    let mut recs = Vec::new();
    let now = crate::journal::now();
    if serv {
        for (root, ks) in serveurs(v, sys::current_session()) {
            if seulement.map(|s| !s.contains(&root.pid)).unwrap_or(false) {
                continue;
            }
            let name = v.raw(root).map(|r| r.name.clone()).unwrap_or_default();
            let (n, b) = tuer_arbre(v, &ks, &mut c);
            if n > 0 {
                c.serveurs += 1;
                c.libere += b;
                recs.push(crate::journal::Rec { t: now, what: "coupé".into(), name, bytes: b });
            }
        }
    }
    if dock {
        let ks = docker_keys(v);
        if !ks.is_empty() {
            let b = v.ram(&ks);
            // proprement d'abord (`docker desktop stop`, Docker Desktop 4.37+), sinon on arrête ses processus
            if sys::run_hidden("docker", &["desktop", "stop"]).is_err() {
                tuer_arbre(v, &ks, &mut c);
            }
            let _ = sys::run_hidden("wsl.exe", &["--terminate", "docker-desktop"]);
            c.docker = true;
            c.libere += b;
            recs.push(crate::journal::Rec { t: now, what: "coupé".into(), name: "Docker Desktop".into(), bytes: b });
        }
    }
    if wsl {
        let vm: u64 = wsl_raws(v).iter().map(|r| r.ws.max(r.private_ws)).sum();
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

/// Ajoute les apps endormies à la liste (fusion avec la précédente : deux modes jeu de suite
/// sans --apres-jeu ne perdent rien), sans doublons ni processus déjà terminés.
pub fn noter_veille(keys: &[Key]) {
    let Some(p) = fichier_veille() else { return };
    let mut v: Vec<Endormi> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let vivants: HashSet<(u32, i64)> = sys::query_processes(&mut Vec::new()).into_iter().map(|r| (r.pid, r.create_time)).collect();
    v.retain(|e| vivants.contains(&(e.pid, e.ctime)));
    for k in keys {
        if !v.iter().any(|e| e.pid == k.pid && e.ctime == k.ctime) {
            v.push(Endormi { pid: k.pid, ctime: k.ctime });
        }
    }
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
        if let Ok(b) = sys::prefetch(e.pid, e.ctime) {
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

#[cfg(test)]
mod couper_tests {
    use super::*;

    fn raw(pid: u32, ppid: u32, name: &str) -> Raw {
        Raw {
            pid,
            ppid,
            name: name.into(),
            create_time: pid as i64 * 10,
            cpu_time: 0,
            private_ws: 50 << 20,
            commit: 0,
            ws: 50 << 20,
            io_read: 0,
            io_write: 0,
            io_other: 0,
            threads: 4,
            handles: 10,
            session: sys::current_session(),
            base_prio: 8,
            suspended: false,
        }
    }

    fn port(p: u16) -> Vec<Port> {
        vec![Port { tcp: true, port: p, local_only: true }]
    }

    /// Sélection de --etat-dev / --couper-dev sur une vue factice (rien n'est tué).
    #[test]
    fn selection_des_serveurs() {
        let raws = vec![
            raw(10, 0, "cmd.exe"),
            raw(11, 10, "node.exe"),   // npm run dev
            raw(12, 11, "node.exe"),   // vite, écoute :5173
            raw(20, 0, "node.exe"),    // serveur avec une fenêtre : non
            raw(30, 0, "node.exe"),    // Claude Code : non
            raw(40, 0, "python.exe"),  // pas de port : non
            raw(50, 0, "node.exe"),    // parent de l'appelant (Vigie = 51) : non
            raw(51, 50, "vigie.exe"),
        ];
        let mut ports = HashMap::new();
        for (pid, p) in [(12, 5173), (20, 3000), (30, 4000), (50, 8080)] {
            ports.insert(pid, port(p));
        }
        let fenetres: HashSet<u32> = [20].into_iter().collect();
        let cmd = |pid: u32| match pid {
            30 => Some(r"node C:\npm\node_modules\@anthropic-ai\claude-code\cli.js".to_string()),
            11 => Some(r"node C:\npm\npm-cli.js run dev".to_string()),
            _ => None,
        };
        let v = Vue::assembler(raws, ports, fenetres, 51, Box::new(cmd));
        let s = serveurs(&v, sys::current_session());
        let racines: Vec<u32> = s.iter().map(|(k, _)| k.pid).collect();
        assert_eq!(racines, vec![11], "seul npm run dev (racine 11) doit être proposé");
        assert_eq!(s[0].1.len(), 2, "arbre = npm + vite");
    }

    /// Vrai arrêt, limité à un node jetable (filtre `seulement`) : les vrais serveurs de
    /// l'utilisateur ne peuvent pas être touchés.
    #[test]
    fn couper_un_faux_serveur() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        let js = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("tmp-couper.js");
        std::fs::write(&js, "require('http').createServer((q,r)=>r.end()).listen(48129,'127.0.0.1')").unwrap();
        let mut child = std::process::Command::new("node").arg(&js).spawn().unwrap();
        let pid = child.id();
        let t0 = std::time::Instant::now();
        while !sys::ports().get(&pid).map(|p| p.iter().any(|x| x.port == 48129)).unwrap_or(false) && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let seul: HashSet<u32> = [pid].into_iter().collect();
        let c = couper_vue(&Vue::lire(), true, false, false, Some(&seul));
        assert_eq!(c.serveurs, 1, "{:?}", c.erreurs);
        assert!(c.libere > 0);
        assert!(child.wait().is_ok());
        let _ = std::fs::remove_file(js);
    }
}
