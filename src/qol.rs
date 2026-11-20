//! Pistes QoL de Vigie (validées) :
//! - **V1** tuer par port (`:3000` dans la palette) ;
//! - **V2** sieste de projet : endormir / réveiller tous les processus d'un projet `C:\Dev\X` ;
//! - **V3** « il te manque X Go pour ce jeu » : pic de RAM retenu pour chaque jeu ;
//! - **V4** boîte noire : les 5 coupables de chaque instant de l'historique, même morts depuis.
//!
//! Tout se calcule sur l'instantané déjà mesuré chaque seconde : aucun appel système en plus.
use crate::sampler::{Key, Proc, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

// ---------- V4 : boîte noire ----------
/// Un coupable à un instant donné.
#[derive(Clone)]
pub struct Culprit {
    pub name: String,
    pub key: Key,
    pub value: f64,
}

/// Les 5 plus gros consommateurs d'une mesure, pour chaque ressource.
#[derive(Clone)]
pub struct Moment {
    pub at: Instant,
    pub cpu: Vec<Culprit>,
    pub mem: Vec<Culprit>,
    pub gpu: Vec<Culprit>,
    pub disk: Vec<Culprit>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Metric {
    Cpu,
    Mem,
    Gpu,
    Disk,
}

impl Moment {
    pub fn list(&self, m: Metric) -> &[Culprit] {
        match m {
            Metric::Cpu => &self.cpu,
            Metric::Mem => &self.mem,
            Metric::Gpu => &self.gpu,
            Metric::Disk => &self.disk,
        }
    }
}

fn top5(procs: &[Proc], f: impl Fn(&Proc) -> f64) -> Vec<Culprit> {
    // sélection partielle : O(n), pas de tri complet de ~400 processus
    let mut best: Vec<(f64, usize)> = Vec::with_capacity(6);
    for (i, p) in procs.iter().enumerate() {
        if p.key.pid <= 4 {
            continue;
        }
        let v = f(p);
        if v <= 0.0 {
            continue;
        }
        if best.len() < 5 || v > best[best.len() - 1].0 {
            let pos = best.iter().position(|b| v > b.0).unwrap_or(best.len());
            best.insert(pos, (v, i));
            best.truncate(5);
        }
    }
    best.into_iter().map(|(v, i)| Culprit { name: procs[i].name.to_string(), key: procs[i].key, value: v }).collect()
}

pub fn moment(snap: &Snapshot) -> Moment {
    let p = &snap.procs;
    Moment {
        at: Instant::now(),
        cpu: top5(p, |x| x.cpu as f64),
        mem: top5(p, |x| x.mem as f64),
        gpu: top5(p, |x| x.gpu as f64),
        disk: top5(p, |x| x.disk),
    }
}

/// Anneau de la même taille que l'historique des graphiques.
pub fn push_moment(ring: &mut VecDeque<Moment>, m: Moment, cap: usize) {
    if ring.len() >= cap {
        ring.pop_front();
    }
    ring.push_back(m);
}

// ---------- V2 : sieste de projet ----------
#[derive(Clone, Debug)]
pub struct Project {
    /// « vigie »
    pub name: String,
    /// « C:\Dev\vigie »
    pub dir: String,
    pub keys: Vec<Key>,
    pub ram: u64,
    pub suspended: usize,
}

impl Project {
    pub fn asleep(&self) -> bool {
        self.suspended > 0 && self.suspended * 2 >= self.keys.len()
    }
}

/// `C:\Dev\x\…` (dans un chemin ou une ligne de commande) → (« x », « C:\Dev\x »).
pub fn project_of(text: &str, root: &str) -> Option<(String, String)> {
    // ASCII seulement : la position trouvée dans `low` doit rester valable dans `text`
    // (to_lowercase peut changer la longueur en octets de certains caractères → panique au découpage)
    let low = text.to_ascii_lowercase();
    let r = root.trim_end_matches('\\').to_ascii_lowercase() + "\\";
    let i = low.find(&r)?;
    let rest = &text[i + r.len()..];
    let name: String = rest.chars().take_while(|c| !matches!(c, '\\' | '/' | '"' | ' ' | '\'')).collect();
    (!name.is_empty() && !name.starts_with('.')).then(|| (name.clone(), format!("{}\\{name}", root.trim_end_matches('\\'))))
}

/// Projets en cours : processus de la session dont l'exe ou la ligne de commande est sous `root`.
/// Exclus : processus avec une fenêtre visible (l'éditeur, le jeu…), protégés, agents IA, Vigie et ses ancêtres.
pub fn projects(snap: &Snapshot, session: u32, root: &str, untouchable: &HashSet<Key>) -> Vec<Project> {
    let mut map: HashMap<String, Project> = HashMap::new();
    for p in &snap.procs {
        if p.session != session || p.key.pid <= 4 || p.window.is_some() || untouchable.contains(&p.key) {
            continue;
        }
        let n = p.name.to_ascii_lowercase();
        if crate::hunter::is_protected_name(&n) && n != "conhost.exe" {
            continue;
        }
        let cmd = p.cmd.as_deref().unwrap_or("");
        if crate::hunter::is_ai_cmd(cmd) {
            continue;
        }
        let Some((name, dir)) = p.path.as_deref().and_then(|x| project_of(x, root)).or_else(|| project_of(cmd, root)) else { continue };
        let e = map.entry(name.to_lowercase()).or_insert_with(|| Project { name, dir, keys: Vec::new(), ram: 0, suspended: 0 });
        e.keys.push(p.key);
        e.ram += p.mem;
        if p.suspended {
            e.suspended += 1;
        }
    }
    let mut v: Vec<Project> = map.into_values().collect();
    v.sort_by(|a, b| b.ram.cmp(&a.ram));
    v
}

/// Vigie et toute sa chaîne de parents (ne jamais endormir celui qui nous a lancés).
pub fn self_and_ancestors(snap: &Snapshot, self_pid: u32) -> HashSet<Key> {
    let by: HashMap<Key, &Proc> = snap.procs.iter().map(|p| (p.key, p)).collect();
    let start = snap.procs.iter().find(|p| p.key.pid == self_pid).map(|p| p.key);
    ancestors(start, |k| by.get(&k).and_then(|p| p.parent))
}

/// `start` et toute sa chaîne de parents (règle « intouchables » partagée par le chasseur,
/// la sieste de projet et `--couper-dev`).
pub fn ancestors(start: Option<Key>, parent: impl Fn(Key) -> Option<Key>) -> HashSet<Key> {
    let mut out = HashSet::new();
    let mut cur = start;
    while let Some(k) = cur {
        if !out.insert(k) {
            break;
        }
        cur = parent(k);
    }
    out
}

// ---------- V3 : pic de RAM des jeux ----------
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct GamePeak {
    /// nom affiché (description de l'exe, sinon nom de l'exe)
    pub name: String,
    /// pic de RAM privée de l'arbre du jeu (octets)
    pub peak: u64,
    /// dernière fois vu (secondes Unix)
    pub seen: u64,
    /// chemin complet de l'exe (pour retrouver un jeu par son dossier d'installation)
    pub path: String,
}

/// Dossiers où vivent les jeux (Steam, Epic, GOG, Riot, EA, Ubisoft, Xbox, Battle.net…).
pub fn is_game_path(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    const MARKS: &[&str] = &[
        "\\steamapps\\common\\",
        "\\epic games\\",
        "\\gog galaxy\\games\\",
        "\\gog games\\",
        "\\riot games\\",
        "\\ea games\\",
        "\\electronic arts\\",
        "\\ubisoft game launcher\\games\\",
        "\\xboxgames\\",
        "\\battle.net\\",
        "\\games\\",
    ];
    // les lanceurs eux-mêmes ne sont pas des jeux
    const NOT: &[&str] = &["launcher", "crashreport", "unitycrashhandler", "easyanticheat", "battleye", "redist", "helper", "webhelper", "epicgameslauncher", "riotclient", "eadesktop", "battle.net.exe", "agent.exe", "ultimate.exe"];
    let file = l.rsplit('\\').next().unwrap_or("");
    MARKS.iter().any(|m| l.contains(m)) && l.ends_with(".exe") && !NOT.iter().any(|n| file.contains(n))
}

/// Clé d'un jeu : nom de l'exe en minuscules.
pub fn game_key(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase()
}

/// Met à jour les pics avec les jeux en cours. Renvoie vrai si un pic a changé (à enregistrer).
pub fn track_games(snap: &Snapshot, peaks: &mut HashMap<String, GamePeak>, desc: impl Fn(&str) -> Option<String>) -> bool {
    // RAM de l'arbre de chaque jeu (un jeu lance parfois des sous-processus)
    let mut kids: HashMap<Key, Vec<&Proc>> = HashMap::new();
    for p in &snap.procs {
        if let Some(par) = p.parent {
            kids.entry(par).or_default().push(p);
        }
    }
    let mut changed = false;
    let now = crate::journal::now();
    for p in &snap.procs {
        let Some(path) = p.path.as_deref() else { continue };
        if !is_game_path(path) {
            continue;
        }
        // seulement la racine du jeu (pas un enfant qui est aussi dans le dossier du jeu)
        if p.parent.and_then(|k| snap.procs.iter().find(|x| x.key == k)).and_then(|pp| pp.path.as_deref()).map(is_game_path).unwrap_or(false) {
            continue;
        }
        let mut ram = p.mem;
        let mut stack: Vec<Key> = vec![p.key];
        let mut n = 0;
        while let Some(k) = stack.pop() {
            n += 1;
            if n > 200 {
                break;
            }
            for c in kids.get(&k).into_iter().flatten() {
                ram += c.mem;
                stack.push(c.key);
            }
        }
        let e = peaks.entry(game_key(path)).or_insert_with(|| GamePeak { name: desc(path).unwrap_or_else(|| p.name.trim_end_matches(".exe").to_string()), peak: 0, seen: now, path: String::new() });
        if e.path != path {
            e.path = path.to_string();
            changed = true;
        }
        // le noyau garde le pic de la racine : un pic bref entre deux mesures (10 s en plein écran)
        // n'est pas perdu. L'arbre, lui, est mesuré à l'instant (les pics des enfants ne coïncident pas).
        let ram = ram.max(p.peak_commit);
        if ram > e.peak {
            e.peak = ram;
            changed = true;
        }
        if now.saturating_sub(e.seen) > 60 {
            e.seen = now;
            changed = true;
        }
    }
    changed
}

/// Jeu connu pour `arg` : nom ou chemin d'un exe, ou **dossier d'installation** (le plus gros pic
/// parmi les exe connus sous ce dossier, ex. `J:\SteamLibrary\steamapps\common\Cyberpunk 2077`).
/// Renvoie (clé de l'exe, pic).
pub fn find_game<'a>(peaks: &'a HashMap<String, GamePeak>, arg: &str) -> Option<(&'a String, &'a GamePeak)> {
    let k = game_key(arg);
    if let Some(e) = peaks.get_key_value(&k).or_else(|| peaks.get_key_value(&format!("{k}.exe"))) {
        return Some(e);
    }
    let dir = arg.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase() + "\\";
    if dir.len() < 4 {
        return None;
    }
    peaks.iter().filter(|(_, g)| g.path.to_lowercase().starts_with(&dir)).max_by_key(|(_, g)| g.peak)
}

/// Ce qu'il manque pour jouer : pic du jeu − RAM disponible (0 si ça passe).
pub fn missing(peak: u64, available: u64) -> u64 {
    // marge de 10 % : Windows et le pilote graphique prennent aussi leur part
    (peak + peak / 10).saturating_sub(available)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projets() {
        assert_eq!(project_of(r#""C:\Program Files\nodejs\node.exe" C:\Dev\Launcher\node_modules\vite\bin\vite.js"#, r"C:\Dev"), Some(("Launcher".into(), r"C:\Dev\Launcher".into())));
        assert_eq!(project_of(r"c:\dev\vigie\target\release\vigie.exe", r"C:\Dev"), Some(("vigie".into(), r"C:\Dev\vigie".into())));
        assert_eq!(project_of(r"C:\Windows\explorer.exe", r"C:\Dev"), None);
    }
    #[test]
    fn jeu_par_dossier() {
        let mut m = HashMap::new();
        m.insert("cyberpunk2077.exe".to_string(), GamePeak { name: "Cyberpunk 2077".into(), peak: 9 << 30, seen: 1, path: r"J:\SteamLibrary\steamapps\common\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe".into() });
        m.insert("redprelauncher.exe".to_string(), GamePeak { name: "x".into(), peak: 1 << 20, seen: 1, path: r"J:\SteamLibrary\steamapps\common\Cyberpunk 2077\REDprelauncher.exe".into() });
        assert_eq!(find_game(&m, r"j:\steamlibrary\steamapps\common\cyberpunk 2077\").map(|(k, _)| k.as_str()), Some("cyberpunk2077.exe"));
        assert_eq!(find_game(&m, "Cyberpunk2077.exe").map(|(_, g)| g.peak), Some(9 << 30));
        assert!(find_game(&m, r"J:\SteamLibrary\steamapps\common\Cyberpunk").is_none());
    }

    #[test]
    fn jeux() {
        assert!(is_game_path(r"D:\SteamLibrary\steamapps\common\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe"));
        assert!(!is_game_path(r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"));
        assert!(!is_game_path(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert_eq!(missing(10 << 30, 12 << 30), 0);
        assert_eq!(missing(10 << 30, 10 << 30), 1 << 30);
    }
}

#[cfg(test)]
mod sieste_tests {
    use crate::app::{Action, App, Settings};
    /// Endort puis réveille un faux projet (node jetable sous C:\Dev\vigie\target), sans toucher à rien d'autre.
    #[test]
    fn endormir_reveiller_projet() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("vigie-test-data");
        let _ = std::fs::create_dir_all(&dir);
        unsafe { std::env::set_var("VIGIE_DATA_DIR", &dir) };
        let js = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("tmp-sieste.js");
        std::fs::write(&js, "setInterval(()=>{},1e6)").unwrap();
        let mut child = std::process::Command::new("node").arg(&js).spawn().unwrap();
        let pid = child.id();
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Settings::default());
        let t0 = std::time::Instant::now();
        let mut found = None;
        while t0.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            app.tick();
            found = app.projects.iter().find(|p| p.keys.iter().any(|k| k.pid == pid)).map(|p| p.dir.clone());
            if found.is_some() {
                break;
            }
        }
        let dir = found.expect("projet du node de test introuvable");
        assert!(dir.to_lowercase().ends_with(r"\dev\vigie"), "{dir}");
        // le processus de test lui-même (sous C:\Dev\vigie\target) ne doit pas être dans le projet
        let me = std::process::id();
        assert!(!app.projects.iter().any(|p| p.keys.iter().any(|k| k.pid == me)));
        let suspended = |pid: u32| crate::sys::query_processes(&mut Vec::new()).into_iter().find(|r| r.pid == pid).map(|r| r.suspended).unwrap_or(false);
        app.actions.push(Action::SleepProject(dir.clone()));
        app.apply_actions();
        assert!(suspended(pid), "node devrait être suspendu — {:?}", app.last_toast());
        app.actions.push(Action::WakeProject(dir));
        app.apply_actions();
        assert!(!suspended(pid), "node devrait être réveillé");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(js);
    }
}
