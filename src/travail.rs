//! Actions longues exécutées hors du thread d'interface (`wsl --shutdown`, `docker stop`, kill
//! d'arbres, purge, plan d'alimentation) : l'interface prépare un **plan** à partir de la mesure,
//! un thread l'exécute, le résultat revient en toast. Plus aucun gel de l'interface.
//!
//! Aussi : l'état du mode jeu (plan d'alimentation à restaurer) dans son propre fichier
//! `%APPDATA%\Vigie\mode-jeu.json`, géré seulement par le mode jeu (plus dans les réglages).
use crate::app::{human_bytes, Settings};
use crate::journal::Rec;
use crate::sampler::Key;
use crate::sys;
use serde::{Deserialize, Serialize};

/// Un processus à terminer (enfants d'abord), avec ce qu'il faut pour l'afficher.
#[derive(Clone, Debug)]
pub struct Cible {
    pub key: Key,
    pub name: String,
    pub mem: u64,
}

/// Un fantôme à chasser.
#[derive(Clone, Debug)]
pub enum Remede {
    /// arbre de processus, enfants d'abord
    Tuer(Vec<Cible>),
    WslShutdown,
    DockerStop(String),
    /// processus Docker Desktop (enfants d'abord), puis `wsl --terminate docker-desktop`
    DockerQuit(Vec<Cible>),
}

#[derive(Clone, Debug)]
pub struct Chasse {
    pub name: String,
    pub ram: u64,
    pub remede: Remede,
}

#[derive(Clone, Debug, Default)]
pub struct PlanJeu {
    pub chasses: Vec<Chasse>,
    /// processus à mettre en veille (vider la RAM sans fermer)
    pub veille: Vec<Cible>,
    pub purge: bool,
    pub power: bool,
}

pub enum Travail {
    Nettoyer(Vec<Chasse>),
    Jeu(PlanJeu),
    /// fin du mode jeu : plan restauré, apps réveillées (préchargement)
    Finir,
}

/// Résultat renvoyé à l'interface.
#[derive(Default, Debug)]
pub struct Fini {
    pub message: String,
    pub erreur: bool,
    pub libere: u64,
    pub journal: Vec<Rec>,
    /// mode jeu : lancé (l'état est enregistré dans mode-jeu.json)
    pub jeu: bool,
}

fn tuer(cibles: &[Cible], errs: &mut Vec<String>) -> (usize, u64) {
    let (mut n, mut b) = (0, 0u64);
    for c in cibles {
        match sys::kill(c.key.pid, c.key.ctime) {
            Ok(()) => {
                n += 1;
                b += c.mem;
            }
            Err(e) if !c.name.eq_ignore_ascii_case("conhost.exe") => errs.push(format!("{} : {e}", c.name)),
            Err(_) => {}
        }
    }
    (n, b)
}

/// Chasse une liste de fantômes. Renvoie (chassés, octets, erreurs, journal).
pub fn chasser(liste: &[Chasse]) -> (usize, u64, Vec<String>, Vec<Rec>) {
    let (mut n, mut freed, mut errs, mut recs) = (0, 0u64, Vec::new(), Vec::new());
    for c in liste {
        let res: Result<u64, String> = match &c.remede {
            Remede::Tuer(cibles) => {
                let (k, b) = tuer(cibles, &mut errs);
                if k > 0 { Ok(b) } else { Err(String::new()) }
            }
            Remede::WslShutdown => sys::run_hidden("wsl.exe", &["--shutdown"]).map(|_| c.ram).map_err(|e| format!("wsl --shutdown : {e}")),
            Remede::DockerStop(id) => crate::docker::stop(id).map(|_| c.ram).map_err(|e| format!("docker stop : {e}")),
            Remede::DockerQuit(cibles) => {
                // proprement d'abord (Docker Desktop 4.37+), sinon on arrête ses processus
                let b = if sys::run_hidden("docker", &["desktop", "stop"]).is_ok() { c.ram } else { tuer(cibles, &mut errs).1 };
                let _ = sys::run_hidden("wsl.exe", &["--terminate", "docker-desktop"]);
                Ok(b.max(c.ram))
            }
        };
        match res {
            Ok(b) => {
                n += 1;
                freed += b;
                recs.push(Rec { t: crate::journal::now(), what: "chassé".into(), name: c.name.clone(), bytes: b });
            }
            Err(e) if !e.is_empty() => errs.push(e),
            Err(_) => {}
        }
    }
    (n, freed, errs, recs)
}

/// Vide la RAM de processus sans les fermer. Renvoie (processus, octets, journal par nom).
pub fn endormir(cibles: &[Cible]) -> (usize, u64, Vec<Rec>) {
    let (mut n, mut freed) = (0, 0u64);
    let mut by: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for c in cibles {
        if let Ok(b) = sys::trim(c.key.pid, c.key.ctime) {
            n += 1;
            freed += b;
            *by.entry(c.name.clone()).or_default() += b;
        }
    }
    let t = crate::journal::now();
    let recs = by.into_iter().filter(|(_, b)| *b > 1 << 20).map(|(name, bytes)| Rec { t, what: "veille".into(), name, bytes }).collect();
    (n, freed, recs)
}

pub fn executer(t: Travail) -> Fini {
    match t {
        Travail::Nettoyer(liste) => {
            let (n, freed, errs, journal) = chasser(&liste);
            let (message, erreur) = if errs.is_empty() {
                (format!("Nettoyage : {n} fantôme(s) chassé(s) · ~{} libérés", human_bytes(freed)), false)
            } else {
                (format!("{n} chassé(s), {} échec(s) : {}", errs.len(), errs[0]), true)
            };
            Fini { message, erreur, libere: freed, journal, jeu: false }
        }
        Travail::Jeu(plan) => jouer(plan),
        Travail::Finir => {
            let (message, erreur, _) = finir();
            Fini { message, erreur, ..Default::default() }
        }
    }
}

fn jouer(plan: PlanJeu) -> Fini {
    let (mut parts, mut errs, mut journal, mut libere) = (Vec::new(), Vec::new(), Vec::new(), 0u64);
    if !plan.chasses.is_empty() {
        let (n, freed, e, r) = chasser(&plan.chasses);
        libere += freed;
        if n > 0 {
            parts.push(format!("{n} fantôme(s), {}", human_bytes(freed)));
        }
        errs.extend(e);
        journal.extend(r);
    }
    if !plan.veille.is_empty() {
        let (n, freed, r) = endormir(&plan.veille);
        // retenus pour le réveil de --apres-jeu / « Quitter le mode jeu »
        crate::cli::noter_veille(&plan.veille.iter().map(|c| c.key).collect::<Vec<_>>());
        libere += freed;
        journal.extend(r);
        if n > 0 && freed > 0 {
            parts.push(format!("{n} app(s) en veille, {}", human_bytes(freed)));
        }
    }
    if plan.purge {
        let before = sys::standby_bytes().unwrap_or(0);
        match sys::purge_standby() {
            Ok(()) => {
                parts.push(format!("cache vidé ({})", human_bytes(before)));
                journal.push(Rec { t: crate::journal::now(), what: "cache".into(), name: "mémoire en veille".into(), bytes: before });
            }
            Err(e) => errs.push(e),
        }
    }
    let mut etat = EtatJeu::load();
    etat.actif = true;
    if plan.power {
        match enter_power() {
            Ok(Some(prev)) => {
                if etat.prev_power.is_none() {
                    etat.prev_power = Some(prev);
                }
                parts.push("plan Performances élevées".into());
            }
            Ok(None) => {}
            Err(e) => errs.push(format!("plan d’alimentation : {e}")),
        }
    }
    etat.save();
    let msg = if parts.is_empty() { "Mode jeu actif — rien à nettoyer".to_string() } else { format!("Mode jeu : {}", parts.join(" · ")) };
    let (message, erreur) = if errs.is_empty() { (msg, false) } else { (format!("{msg} — {}", errs[0]), true) };
    Fini { message, erreur, libere, journal, jeu: true }
}

/// Passe en Performances élevées ; renvoie le plan précédent (à restaurer).
/// Ne rétrograde jamais un plan déjà au maximum (Ultimate et ses copies). Aucun powercfg sauf
/// pour recréer le plan s'il est masqué.
pub fn enter_power() -> Result<Option<String>, String> {
    let Some((prev, _, max)) = sys::active_power_info() else { return Err("plan actif illisible".into()) };
    if max {
        return Ok(None);
    }
    if sys::set_power_scheme(sys::HIGH_PERF).is_err() {
        // plan masqué sur certains PC : on le recrée depuis le modèle Windows
        let _ = sys::run_hidden("powercfg.exe", &["-duplicatescheme", sys::HIGH_PERF, sys::HIGH_PERF]);
        sys::set_power_scheme(sys::HIGH_PERF)?;
    }
    Ok(Some(prev))
}

/// Fin du mode jeu : restaure le plan d'alimentation et réveille les apps mises en veille.
/// Partagé par l'interface et `--apres-jeu`. Renvoie (message, erreur, apps réveillées).
pub fn finir() -> (String, bool, usize) {
    let mut parts = vec!["Mode jeu terminé".to_string()];
    let mut err = false;
    let mut etat = EtatJeu::load();
    if let Some(prev) = etat.prev_power.take() {
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
    etat.actif = false;
    etat.save();
    (parts.join(" · "), err, n)
}

// ---------- état du mode jeu (fichier à part) ----------
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct EtatJeu {
    pub actif: bool,
    /// plan d'alimentation à restaurer en quittant le mode jeu
    pub prev_power: Option<String>,
}

impl EtatJeu {
    fn path() -> Option<std::path::PathBuf> {
        Settings::dir().map(|d| d.join("mode-jeu.json"))
    }
    pub fn load() -> EtatJeu {
        Self::path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            let _ = socle::settings::save_to(&p, self);
        }
    }
    /// Reprend un `prev_power` resté dans l'ancien settings.json (versions précédentes).
    pub fn migrer(ancien: Option<String>) {
        if let Some(prev) = ancien {
            let mut e = Self::load();
            if e.prev_power.is_none() {
                e.prev_power = Some(prev);
                e.actif = true;
                e.save();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mode jeu sur un processus jetable : mise en veille, veille.json fusionné (pas écrasé),
    /// état écrit dans mode-jeu.json. Ni purge ni plan d'alimentation (réglages de l'utilisateur).
    #[test]
    fn jeu_sur_faux_processus_et_fusion_veille() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-data-jeu");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("VIGIE_DATA_DIR", &dir) };
        let mut kids: Vec<std::process::Child> = (0..2)
            .map(|_| std::process::Command::new("powershell").args(["-NoProfile", "-Command", "$b = New-Object byte[] 30000000; for($i=0;$i -lt $b.Length;$i+=4096){$b[$i]=1}; Start-Sleep 20"]).spawn().unwrap())
            .collect();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let raws = sys::query_processes(&mut Vec::new());
        let cible = |c: &std::process::Child| {
            let r = raws.iter().find(|r| r.pid == c.id()).unwrap();
            Cible { key: Key { pid: r.pid, ctime: r.create_time }, name: r.name.clone(), mem: r.private_ws }
        };
        // deux modes jeu de suite, une app chacun : les deux doivent rester dans veille.json
        for k in &kids {
            let f = executer(Travail::Jeu(PlanJeu { veille: vec![cible(k)], ..Default::default() }));
            assert!(f.jeu && !f.erreur, "{}", f.message);
        }
        let v: Vec<crate::cli::Endormi> = serde_json::from_slice(&std::fs::read(dir.join("veille.json")).unwrap()).unwrap();
        assert_eq!(v.len(), 2, "veille.json doit fusionner, pas écraser");
        assert!(EtatJeu::load().actif);
        let f = executer(Travail::Finir);
        assert!(f.message.contains("2 app(s) réveillée(s)"), "{}", f.message);
        assert!(!EtatJeu::load().actif);
        for k in &mut kids {
            let _ = k.kill();
            let _ = k.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
