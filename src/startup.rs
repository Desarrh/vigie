//! Apps lancées avec Windows : clés Run (utilisateur et machine) et dossiers Démarrage,
//! activées / désactivées comme le fait le Gestionnaire des tâches (clés `StartupApproved`).
use crate::sys::wide;
use windows_sys::Win32::System::Registry::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// HKCU\...\Run
    UserRun,
    /// HKLM\...\Run
    MachineRun,
    /// HKLM\...\WOW6432Node\...\Run
    MachineRun32,
    /// dossier Démarrage de l'utilisateur
    UserFolder,
    /// dossier Démarrage commun
    CommonFolder,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::UserRun => "Registre (utilisateur)",
            Source::MachineRun => "Registre (tous les utilisateurs)",
            Source::MachineRun32 => "Registre 32 bits (tous)",
            Source::UserFolder => "Dossier Démarrage",
            Source::CommonFolder => "Dossier Démarrage (tous)",
        }
    }
    /// Modifier l'état demande l'admin ?
    pub fn needs_admin(self) -> bool {
        matches!(self, Source::MachineRun | Source::MachineRun32 | Source::CommonFolder)
    }
    fn approved(self) -> (HKEY, &'static str) {
        match self {
            Source::UserRun => (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"),
            Source::MachineRun => (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"),
            Source::MachineRun32 => (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32"),
            Source::UserFolder => (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder"),
            Source::CommonFolder => (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// nom de la valeur (registre) ou du fichier (dossier)
    pub name: String,
    pub command: String,
    /// exe déduit de la commande (minuscules), pour retrouver les processus lancés
    pub exe: Option<String>,
    pub source: Source,
    pub enabled: bool,
}

fn open(root: HKEY, path: &str, access: u32) -> Option<HKEY> {
    let mut h: HKEY = std::ptr::null_mut();
    (unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, access, &mut h) } == 0).then_some(h)
}

/// Valeurs texte d'une clé : (nom, données).
fn string_values(root: HKEY, path: &str, wow64_32: bool) -> Vec<(String, String)> {
    let access = KEY_READ | if wow64_32 { KEY_WOW64_32KEY } else { KEY_WOW64_64KEY };
    let Some(h) = open(root, path, access) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0.. {
        let mut name = [0u16; 512];
        let mut nlen = name.len() as u32;
        let mut data = vec![0u8; 8192];
        let mut dlen = data.len() as u32;
        let mut ty = 0u32;
        let r = unsafe { RegEnumValueW(h, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut ty, data.as_mut_ptr(), &mut dlen) };
        if r != 0 {
            break;
        }
        if ty == REG_SZ || ty == REG_EXPAND_SZ {
            let w: Vec<u16> = data[..dlen as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
            out.push((String::from_utf16_lossy(&name[..nlen as usize]), expand(&String::from_utf16_lossy(&w[..n]))));
        }
    }
    unsafe { RegCloseKey(h) };
    out
}

fn expand(s: &str) -> String {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    if !s.contains('%') {
        return s.to_string();
    }
    let src = wide(s);
    let mut buf = vec![0u16; 2048];
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 || n as usize > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)])
}

/// État dans StartupApproved : octet 0 pair = activé (absent = activé).
fn approved_state(src: Source, name: &str) -> bool {
    let (root, path) = src.approved();
    let mut data = [0u8; 12];
    let mut len = data.len() as u32;
    let r = unsafe { RegGetValueW(root, wide(path).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut len) };
    if r != 0 || len == 0 {
        return true;
    }
    data[0] & 1 == 0
}

pub fn set_enabled(e: &Entry, on: bool) -> Result<(), String> {
    let (root, path) = e.source.approved();
    let mut data = [0u8; 12];
    if on {
        data[0] = 0x02;
    } else {
        data[0] = 0x03;
        // horodatage de la désactivation (affiché par le Gestionnaire des tâches)
        let mut f = windows_sys::Win32::Foundation::FILETIME::default();
        unsafe { windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime(&mut f) };
        data[4..8].copy_from_slice(&f.dwLowDateTime.to_le_bytes());
        data[8..12].copy_from_slice(&f.dwHighDateTime.to_le_bytes());
    }
    let r = unsafe { RegSetKeyValueW(root, wide(path).as_ptr(), wide(&e.name).as_ptr(), REG_BINARY, data.as_ptr().cast(), 12) };
    if r == 0 {
        Ok(())
    } else if r == 5 {
        Err("accès refusé — relance Vigie en admin pour les entrées « tous les utilisateurs »".into())
    } else {
        Err(std::io::Error::from_raw_os_error(r as i32).to_string())
    }
}

/// Exe d'une ligne de commande : `"C:\a b\x.exe" --arg` ou `C:\x.exe arg`.
pub fn exe_of(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    let path = if let Some(r) = c.strip_prefix('"') {
        r.split('"').next()?.to_string()
    } else {
        let low = c.to_ascii_lowercase(); // même longueur que `c` : le découpage reste valide
        match low.find(".exe") {
            Some(i) => c[..i + 4].to_string(),
            None => c.split(' ').next()?.to_string(),
        }
    };
    Some(path.to_lowercase())
}

pub fn list() -> Vec<Entry> {
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let mut out = Vec::new();
    for (src, root, wow) in [(Source::UserRun, HKEY_CURRENT_USER, false), (Source::MachineRun, HKEY_LOCAL_MACHINE, false), (Source::MachineRun32, HKEY_LOCAL_MACHINE, true)] {
        for (name, command) in string_values(root, RUN, wow) {
            let enabled = approved_state(src, &name);
            out.push(Entry { exe: exe_of(&command), name, command, source: src, enabled });
        }
    }
    let folders = [
        (Source::UserFolder, crate::sys::appdata().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs\Startup"))),
        (Source::CommonFolder, std::env::var_os("ProgramData").map(|d| std::path::PathBuf::from(d).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))),
    ];
    for (src, dir) in folders {
        let Some(dir) = dir else { continue };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let path = e.path();
            // raccourci : on ne lit pas la cible (COM), le nom suffit à retrouver l'app
            let exe = if path.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false) { Some(path.to_string_lossy().to_lowercase()) } else { None };
            let enabled = approved_state(src, &name);
            out.push(Entry { name, command: path.to_string_lossy().into_owned(), exe, source: src, enabled });
        }
    }
    out.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// RAM actuelle de chaque exe de démarrage : processus du même exe, ou exe du même nom sous le
/// dossier du lanceur (Discord : `Discord\app-1.0\Discord.exe`). Calculée à la mesure.
pub fn ram_table(list: &[Entry], procs: &[crate::sampler::Proc]) -> std::collections::HashMap<String, (u64, usize)> {
    let paths: Vec<(String, u64)> = procs.iter().filter_map(|p| p.path.as_ref().map(|x| (x.to_lowercase(), p.mem))).collect();
    let mut ram = std::collections::HashMap::new();
    for e in list {
        let Some(exe) = &e.exe else { continue };
        let pe = std::path::Path::new(exe);
        let dir = pe.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let file = pe.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let (pre, suf) = (format!("{dir}\\"), format!("\\{file}"));
        let mut acc = (0u64, 0usize);
        for (path, mem) in &paths {
            if path == exe || (!dir.is_empty() && path.starts_with(&pre) && path.ends_with(&suf)) {
                acc.0 += mem;
                acc.1 += 1;
            }
        }
        if acc.1 > 0 {
            ram.insert(exe.clone(), acc);
        }
    }
    ram
}

#[cfg(test)]
mod tests {
    #[test]
    fn exe_parsing() {
        assert_eq!(super::exe_of(r#""C:\Program Files\A B\x.exe" --y"#).as_deref(), Some(r"c:\program files\a b\x.exe"));
        assert_eq!(super::exe_of(r"C:\Riot Games\Riot Client\RiotClientServices.exe --launch").as_deref(), Some(r"c:\riot games\riot client\riotclientservices.exe"));
    }
    #[test]
    fn lists_something() {
        let l = super::list();
        eprintln!("{} entrées : {:?}", l.len(), l.iter().take(3).map(|e| (&e.name, e.enabled)).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::*;
    /// Écrit (puis efface) une entrée factice dans le registre de démarrage de l'utilisateur :
    /// désactivé par défaut (règle n°0). `cargo test -- --ignored` seulement avec son accord.
    #[test]
    #[ignore]
    fn toggle_fake_entry() {
        const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
        let v = wide(r"C:\nexiste\pas\VigieTest.exe");
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide("VigieTest").as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) };
        let find = || list().into_iter().find(|e| e.name == "VigieTest").unwrap();
        let e = find();
        assert!(e.enabled);
        set_enabled(&e, false).unwrap();
        assert!(!find().enabled);
        set_enabled(&e, true).unwrap();
        assert!(find().enabled);
        unsafe {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide("VigieTest").as_ptr());
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(Source::UserRun.approved().1).as_ptr(), wide("VigieTest").as_ptr());
        }
        assert!(list().iter().all(|e| e.name != "VigieTest"));
    }
}
