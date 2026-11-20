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
