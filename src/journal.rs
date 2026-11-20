//! Journal des nettoyages : une ligne JSON par action dans %APPDATA%\Vigie\journal.jsonl.
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Rec {
    /// secondes Unix
    pub t: u64,
    /// « tué », « chassé », « veille », « cache », « docker »
    pub what: String,
    pub name: String,
    pub bytes: u64,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn path() -> Option<std::path::PathBuf> {
    crate::app::Settings::dir().map(|d| d.join("journal.jsonl"))
}

pub fn append(recs: &[Rec]) {
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        for r in recs {
            if let Ok(s) = serde_json::to_string(r) {
                let _ = writeln!(f, "{s}");
            }
        }
    }
}

/// Entrées des `days` derniers jours. Au-delà de 90 jours, le fichier est compacté.
pub fn load(days: u64) -> Vec<Rec> {
    let Some(p) = path() else { return Vec::new() };
    let Ok(s) = std::fs::read_to_string(&p) else { return Vec::new() };
    let all: Vec<Rec> = s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let n = now();
    let keep: Vec<&Rec> = all.iter().filter(|r| n.saturating_sub(r.t) < 90 * 86_400).collect();
    if keep.len() < all.len() {
        let txt: String = keep.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
        let _ = std::fs::write(&p, txt);
    }
    all.into_iter().filter(|r| n.saturating_sub(r.t) < days * 86_400).collect()
}

/// Relevé d'inactivité : temps CPU de chaque processus au moment de l'enregistrement
/// (fermeture de Vigie, et toutes les 5 min en cas de plantage). Au lancement suivant, un
/// processus au même temps CPU est inactif depuis au moins ce moment-là. Rien ne tourne
/// pendant que Vigie est fermée.
#[derive(Serialize, Deserialize, Default)]
pub struct Releve {
    /// secondes Unix de l'enregistrement
    pub t: u64,
    /// (pid, date de création, temps CPU)
    pub p: Vec<(u32, i64, i64)>,
}

impl Releve {
    fn path() -> Option<std::path::PathBuf> {
        crate::app::Settings::dir().map(|d| d.join("releve.json"))
    }
    pub fn save(procs: &[crate::sampler::Proc]) {
        let Some(p) = Self::path() else { return };
        let r = Releve { t: now(), p: procs.iter().filter(|x| x.key.pid > 4).map(|x| (x.key.pid, x.key.ctime, x.cpu_time)).collect() };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        if let Ok(b) = serde_json::to_vec(&r) {
            let _ = std::fs::write(p, b);
        }
    }
    /// (instant de l'enregistrement, (pid, ctime) → temps CPU)
    pub fn load() -> Option<(u64, std::collections::HashMap<(u32, i64), i64>)> {
        let r: Releve = serde_json::from_slice(&std::fs::read(Self::path()?).ok()?).ok()?;
        Some((r.t, r.p.into_iter().map(|(pid, ct, cpu)| ((pid, ct), cpu)).collect()))
    }
}

pub fn ago(t: u64) -> String {
    let d = now().saturating_sub(t);
    if d < 60 {
        "à l’instant".into()
    } else if d < 3600 {
        format!("il y a {} min", d / 60)
    } else if d < 86_400 {
        format!("il y a {} h", d / 3600)
    } else {
        format!("il y a {} j", d / 86_400)
    }
}
