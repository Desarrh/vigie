//! Chasseur de RAM fantôme : ce qui occupe de la mémoire sans servir à rien.
//!
//! Règles (prudentes) — seuls les processus de la session de l'utilisateur sont examinés,
//! jamais les services ni le système :
//! - **Serveur de dev oublié** : outil de dev (node, python, cargo…) qui écoute sur un port.
//! - **Orphelin** : outil de dev dont le parent (terminal, éditeur) a disparu.
//! - **Démon de build** : VBCSCompiler, MSBuild, Gradle, Kotlin… inactifs, ils se relancent seuls.
//! - **App en fond** : app connue (Edge en pré-chargement, Widgets, Teams…) sans aucune fenêtre.
//! - **Lourd et inactif** : arbre sans fenêtre, > 250 Mo, aucune activité depuis 10 min.
//! - **VM WSL / Docker** : vmmem, arrêtable avec `wsl --shutdown`.
//!
//! « Sûr » (pré-coché) seulement quand l'arrêt ne peut rien casser d'utile.
use crate::sampler::{Key, Proc, Snapshot};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    DevServer,
    Orphan,
    BuildDaemon,
    Background,
    HeavyIdle,
    Docker,
    Wsl,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::DevServer => "Serveurs de dev",
            Kind::Orphan => "Orphelins",
            Kind::BuildDaemon => "Démons de build",
            Kind::Background => "Apps en fond",
            Kind::HeavyIdle => "Lourds et inactifs",
            Kind::Docker => "Docker",
            Kind::Wsl => "Machines virtuelles",
        }
    }
    pub fn icon(self) -> socle::icons::Icon {
        use socle::icons::Icon;
        match self {
            Kind::DevServer => Icon::Code,
            Kind::Orphan => Icon::Block,
            Kind::BuildDaemon => Icon::Settings,
            Kind::Background => Icon::Eye,
            Kind::HeavyIdle => Icon::Sleep,
            Kind::Docker => Icon::Container,
            Kind::Wsl => Icon::Terminal,
        }
    }
    pub const ORDER: [Kind; 7] = [Kind::DevServer, Kind::Docker, Kind::Orphan, Kind::BuildDaemon, Kind::Background, Kind::HeavyIdle, Kind::Wsl];
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fix {
    /// tuer l'arbre (processus + descendants)
    KillTree,
    /// `wsl --shutdown`
    WslShutdown,
    /// `docker stop <id>`
    DockerStop(String),
    /// quitter Docker Desktop et arrêter sa VM (`wsl --terminate docker-desktop`)
    DockerQuit,
}

#[derive(Clone)]
pub struct Ghost {
    pub key: Key,
    pub name: String,
    pub kind: Kind,
    pub reason: String,
    /// RAM récupérable : plage de travail privée de tout l'arbre
    pub ram: u64,
    /// nombre de processus dans l'arbre
    pub count: usize,
    pub safe: bool,
    pub fix: Fix,
    /// règle de liste blanche proposée (« node.exe » ou « node.exe | server.js »)
    pub rule: String,
    pub cmd: Option<String>,
}

const DEV_TOOLS: &[&str] = &[
    "node.exe", "bun.exe", "deno.exe", "python.exe", "python3.exe", "pythonw.exe", "py.exe", "java.exe", "javaw.exe",
    "dotnet.exe", "ruby.exe", "php.exe", "php-cgi.exe", "go.exe", "cargo.exe", "rustc.exe", "cargo-watch.exe",
    "esbuild.exe", "uvicorn.exe", "gunicorn.exe", "flask.exe", "hugo.exe", "caddy.exe", "nginx.exe", "httpd.exe",
    "redis-server.exe", "mongod.exe", "ngrok.exe", "cloudflared.exe", "air.exe", "trunk.exe", "wasm-pack.exe",
    "rust-analyzer.exe", "gopls.exe", "clangd.exe", "tsserver.exe", "npm.exe", "pnpm.exe", "yarn.exe",
    "vite.exe", "turbo.exe", "electron.exe", "godot.exe", "adb.exe", "gradle.exe", "mvn.exe",
];

/// Démons de compilation conçus pour être tués et relancés à la demande.
const BUILD_DAEMONS: &[(&str, &str)] = &[
    ("vbcscompiler.exe", "serveur de compilation C# (Roslyn)"),
    ("msbuild.exe", "nœud MSBuild réutilisable"),
    ("mspdbsrv.exe", "serveur de symboles MSVC"),
    ("sccache.exe", "cache de compilation"),
];

/// Apps connues qui tournent souvent sans fenêtre. (nom, description, sûr si sans fenêtre)
const BACKGROUND_APPS: &[(&str, &str, bool)] = &[
    ("msedge.exe", "Edge en pré-chargement (démarrage rapide)", true),
    ("widgets.exe", "Widgets Windows", true),
    ("widgetservice.exe", "Widgets Windows", true),
    ("phoneexperiencehost.exe", "Lien avec le téléphone", true),
    ("ms-teams.exe", "Teams en arrière-plan", false),
    ("teams.exe", "Teams (classique) en arrière-plan", false),
    ("onedrive.exe", "OneDrive (synchronisation)", false),
    ("spotify.exe", "Spotify sans fenêtre", false),
    ("adobecollabsync.exe", "Adobe Acrobat (synchro)", true),
    ("ccxprocess.exe", "Adobe Creative Cloud", true),
    ("coresync.exe", "Adobe Creative Cloud", true),
    ("adobe desktop service.exe", "Adobe Creative Cloud", true),
    ("adobeipcbroker.exe", "Adobe Creative Cloud", true),
    ("creative cloud.exe", "Adobe Creative Cloud", false),
    ("epicgameslauncher.exe", "Epic Games Launcher sans fenêtre", false),
    ("eadesktop.exe", "EA app sans fenêtre", false),
    ("ubisoftconnect.exe", "Ubisoft Connect sans fenêtre", false),
    ("battle.net.exe", "Battle.net sans fenêtre", false),
    ("googledrivefs.exe", "Google Drive", false),
    ("dropbox.exe", "Dropbox", false),
    ("slack.exe", "Slack sans fenêtre", false),
    ("whatsapp.exe", "WhatsApp sans fenêtre", false),
    ("copilot.exe", "Copilot sans fenêtre", true),
    ("cortana.exe", "Cortana", true),
];

/// Jamais proposés, quoi qu'il arrive.
const PROTECTED: &[&str] = &[
    "system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe",
    "svchost.exe", "dwm.exe", "explorer.exe", "sihost.exe", "ctfmon.exe", "fontdrvhost.exe", "audiodg.exe",
    "taskhostw.exe", "runtimebroker.exe", "searchhost.exe", "startmenuexperiencehost.exe", "shellexperiencehost.exe",
    "textinputhost.exe", "conhost.exe", "openconsole.exe", "windowsterminal.exe", "msmpeng.exe", "securityhealthsystray.exe",
    "dllhost.exe", "applicationframehost.exe", "lockapp.exe", "userinit.exe", "smartscreen.exe", "nvcontainer.exe",
    "nvidia overlay.exe", "rtkaudueservice64.exe", "steam.exe", "steamwebhelper.exe", "discord.exe",
    "vigie.exe", "code.exe", "claude.exe", "cursor.exe", "devenv.exe", "rider64.exe", "idea64.exe",
];

/// Processus Docker Desktop (hors VM)
/// Processus de Docker Desktop (hors VM), partagés par le chasseur, `--etat-dev` et `--couper-dev`.
pub const DOCKER_PROCS: &[&str] = &["docker desktop.exe", "com.docker.backend.exe", "com.docker.build.exe", "com.docker.dev-envs.exe", "docker-sandbox.exe"];

/// Jamais touché par Vigie (chasse, mise en veille).
pub fn is_protected_name(lower: &str) -> bool {
    PROTECTED.contains(&lower)
}

pub fn is_dev_tool(name: &str) -> bool {
    DEV_TOOLS.contains(&name)
}

/// Le processus fait-il partie d'un agent IA / outil qu'il ne faut jamais tuer (Claude Code, etc.) ?
fn is_ai_agent(p: &Proc) -> bool {
    is_ai_cmd(p.cmd.as_deref().unwrap_or(""))
}

/// Agent IA ou outil d'éditeur qu'il ne faut jamais tuer ni endormir : Claude Code, Claude,
/// Copilot, Codex. Même règle dans le chasseur, la sieste de projet et `--couper-dev`.
pub fn is_ai_cmd(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    ["@anthropic-ai", "claude-code", r"\claude.exe", "copilot-language-server", "github.copilot", "@openai/codex", r"\codex.exe"].iter().any(|m| c.contains(m))
}

pub fn is_docker_name(lower: &str) -> bool {
    DOCKER_PROCS.contains(&lower)
}

/// Règle de liste blanche qui couvre ce processus : `nom` ou `nom | fragment de ligne de commande`.
pub fn matches_rule(rule: &str, name: &str, cmd: Option<&str>) -> bool {
    let (rn, rc) = match rule.split_once('|') {
        Some((n, c)) => (n.trim(), Some(c.trim())),
        None => (rule.trim(), None),
    };
    if !rn.eq_ignore_ascii_case(name) {
        return false;
    }
    match rc {
        None => true,
        Some(c) => cmd.map(|s| s.to_ascii_lowercase().contains(&c.to_ascii_lowercase())).unwrap_or(false),
    }
}

/// Fragment distinctif de la ligne de commande (le script lancé), pour une liste blanche précise.
fn signature(p: &Proc) -> Option<String> {
    let cmd = p.cmd.as_deref()?;
    let args: Vec<&str> = split_args(cmd);
    args.iter()
        .skip(1)
        .find(|a| !a.starts_with('-') && (a.contains('\\') || a.contains('/') || a.contains('.')))
        .map(|a| {
            let a = a.trim_matches('"');
            std::path::Path::new(a).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| a.to_string())
        })
}

pub fn split_args(cmd: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = cmd.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        let mut q = false;
        while i < b.len() && (q || b[i] != b' ') {
            if b[i] == b'"' {
                q = !q;
            }
            i += 1;
        }
        out.push(&cmd[start..i]);
    }
    out
}

pub fn human_idle(s: f32) -> String {
    let m = (s / 60.0) as u64;
    if m < 1 {
        format!("{} s", s as u64)
    } else if m < 60 {
        format!("{m} min")
    } else if m < 48 * 60 {
        format!("{} h {:02}", m / 60, m % 60)
    } else {
        format!("{} j", m / 1440)
    }
}

/// « 12 min », ou « ≈ 3 h » quand c'est une estimation (processus déjà là à l'ouverture de Vigie).
pub fn idle_text(p: &Proc) -> String {
    if p.idle_min { format!("≈ {}", human_idle(p.idle_secs)) } else { human_idle(p.idle_secs) }
}

pub struct Context<'a> {
    pub session: u32,
    pub self_pid: u32,
    pub whitelist: &'a [String],
    pub docker: &'a [crate::docker::Container],
}

pub fn hunt(snap: &Snapshot, cx: &Context) -> Vec<Ghost> {
    let by_key: HashMap<Key, &Proc> = snap.procs.iter().map(|p| (p.key, p)).collect();
    let mut children: HashMap<Key, Vec<Key>> = HashMap::new();
    for p in &snap.procs {
        if let Some(par) = p.parent {
            children.entry(par).or_default().push(p.key);
        }
    }
    let lname = |p: &Proc| p.name.to_ascii_lowercase();
    // ancêtres de Vigie (ex. Claude Code qui l'a lancée) : intouchables
    let protected: HashSet<Key> = crate::qol::self_and_ancestors(snap, cx.self_pid);
    let tree = |k: Key| -> Vec<Key> {
        let mut out = vec![k];
        let mut i = 0;
        while i < out.len() {
            if let Some(c) = children.get(&out[i]) {
                out.extend(c.iter().copied());
            }
            i += 1;
            if out.len() > 5000 {
                break;
            }
        }
        out
    };
    let tree_has_window = |ks: &[Key]| ks.iter().any(|k| by_key.get(k).map(|p| p.window.is_some()).unwrap_or(false));
    let tree_ram = |ks: &[Key]| ks.iter().filter_map(|k| by_key.get(k)).map(|p| p.mem).sum::<u64>();
    let tree_idle = |ks: &[Key]| ks.iter().filter_map(|k| by_key.get(k)).map(|p| p.idle_secs).fold(f32::MAX, f32::min);
    let tree_idle_txt = |ks: &[Key]| {
        let ps: Vec<&&Proc> = ks.iter().filter_map(|k| by_key.get(k)).collect();
        let secs = ps.iter().map(|p| p.idle_secs).fold(f32::MAX, f32::min);
        if ps.iter().all(|p| p.idle_min) { format!("≈ {}", human_idle(secs)) } else { human_idle(secs) }
    };
    let tree_safe_from_protected = |ks: &[Key]| {
        ks.iter().all(|k| {
            !protected.contains(k)
                && by_key
                    .get(k)
                    .map(|p| {
                        let n = lname(p);
                        // la console attachée (conhost) meurt avec son processus : pas une raison de bloquer
                        let console = (n == "conhost.exe" || n == "openconsole.exe") && k != &ks[0];
                        console || (!PROTECTED.contains(&n.as_str()) && !is_ai_agent(p))
                    })
                    .unwrap_or(true)
        })
    };
    let parent_name = |p: &Proc| p.parent.and_then(|k| by_key.get(&k)).map(|pp| pp.name.to_string());

    let mut ghosts: Vec<Ghost> = Vec::new();
    let mut covered: HashSet<Key> = HashSet::new();
    let mut push = |g: Ghost, ks: &[Key], covered: &mut HashSet<Key>| {
        covered.extend(ks.iter().copied());
        ghosts.push(g);
    };
    let white = |p: &Proc| cx.whitelist.iter().any(|r| matches_rule(r, &p.name, p.cmd.as_deref()));

    // Racines : on remonte au plus haut ancêtre qui est lui aussi un outil de dev (npm → node → esbuild).
    let dev_root = |p: &Proc| -> Key {
        let mut k = p.key;
        while let Some(par) = by_key.get(&k).and_then(|q| q.parent) {
            match by_key.get(&par) {
                Some(pp) if is_dev_tool(&lname(pp)) || lname(pp) == "cmd.exe" && pp.window.is_none() && pp.parent.is_none() => k = par,
                _ => break,
            }
        }
        k
    };

    // Docker : conteneurs inactifs, puis Docker Desktop lui-même s'il ne sert à rien
    let docker_keys: Vec<Key> = snap.procs.iter().filter(|p| DOCKER_PROCS.contains(&lname(p).as_str())).map(|p| p.key).collect();
    if !docker_keys.is_empty() && !cx.whitelist.iter().any(|r| r.eq_ignore_ascii_case("docker")) {
        for c in cx.docker {
            if c.idle_secs > 600.0 || (c.idle_min && c.cpu < 0.5) {
                let mut h: u64 = 1469598103934665603;
                for b in c.id.bytes() {
                    h = (h ^ b as u64).wrapping_mul(1099511628211);
                }
                push(Ghost {
                    key: Key { pid: 0, ctime: h as i64 },
                    name: c.name.clone(),
                    kind: Kind::Docker,
                    reason: format!("conteneur {} · inactif depuis {}{}", c.image, if c.idle_min { "≈ " } else { "" }, human_idle(c.idle_secs)),
                    ram: c.mem,
                    count: 1,
                    safe: false,
                    fix: Fix::DockerStop(c.id.clone()),
                    rule: "docker".into(),
                    cmd: None,
                }, &[], &mut covered);
            }
        }
        let active = cx.docker.iter().any(|c| c.idle_secs < 600.0 && !(c.idle_min && c.cpu < 0.5));
        if !active {
            let mut ks: Vec<Key> = Vec::new();
            for &k in &docker_keys {
                ks.extend(tree(k));
            }
            ks.sort();
            ks.dedup();
            let ram = tree_ram(&ks);
            let vm = snap.procs.iter().find(|p| lname(p).starts_with("vmmemwsl")).map(|p| p.ws.max(p.mem)).unwrap_or(0);
            push(Ghost {
                key: docker_keys[0],
                name: "Docker Desktop".into(),
                kind: Kind::Docker,
                reason: if cx.docker.is_empty() {
                    "aucun conteneur en cours · quitte l’app et arrête sa VM".into()
                } else {
                    format!("{} conteneur(s), tous inactifs · quitte l’app et arrête sa VM", cx.docker.len())
                },
                ram: ram + if cx.docker.is_empty() { vm / 2 } else { 0 },
                count: ks.len(),
                safe: false,
                fix: Fix::DockerQuit,
                rule: "docker".into(),
                cmd: None,
            }, &ks, &mut covered);
        } else {
            for &k in &docker_keys {
                covered.extend(tree(k));
            }
        }
    }

    // WSL / Docker (hors session utilisateur : traité à part)
    for p in &snap.procs {
        let n = lname(p);
        if (n == "vmmemwsl" || n == "vmmemwsl.exe") && p.mem + p.ws > 200 << 20 && !white(p) {
            let ram = p.ws.max(p.mem);
            push(
                Ghost {
                    key: p.key,
                    name: p.name.to_string(),
                    kind: Kind::Wsl,
                    reason: format!("VM WSL · {}", if p.idle_secs < 2.0 { "active".to_string() } else { format!("inactive depuis {}", idle_text(p)) }),
                    ram,
                    count: 1,
                    safe: false,
                    fix: Fix::WslShutdown,
                    rule: p.name.to_string(),
                    cmd: None,
                },
                &[p.key],
                &mut covered,
            );
        }
    }

    let mut cands: Vec<&Proc> = snap.procs.iter().filter(|p| p.session == cx.session && p.key.pid > 4).collect();
    cands.sort_by_key(|p| p.key.ctime);

    for p in &cands {
        let n = lname(p);
        if covered.contains(&p.key) || protected.contains(&p.key) {
            continue;
        }
        if PROTECTED.contains(&n.as_str()) || is_ai_agent(p) || white(p) {
            continue;
        }
        let dev = is_dev_tool(&n);
        // 1) serveur de dev : outil de dev qui écoute sur un port TCP
        if dev && p.ports.iter().any(|x| x.tcp) && p.window.is_none() {
            let root = dev_root(p);
            if covered.contains(&root) {
                continue;
            }
            let ks = tree(root);
            if !tree_safe_from_protected(&ks) || tree_has_window(&ks) {
                continue;
            }
            let rp = by_key.get(&root).copied().unwrap_or(p);
            let orphan = rp.parent.is_none();
            let idle = tree_idle(&ks);
            let ports: Vec<String> = ks
                .iter()
                .filter_map(|k| by_key.get(k))
                .flat_map(|q| q.ports.iter().filter(|x| x.tcp).map(|x| format!(":{}", x.port)))
                .take(4)
                .collect();
            let origin = match parent_name(rp) {
                Some(pn) => format!("lancé depuis {pn}"),
                None => "terminal fermé".into(),
            };
            let sig = signature(rp);
            push(
                Ghost {
                    key: root,
                    name: rp.name.to_string(),
                    kind: Kind::DevServer,
                    reason: format!("écoute {} · inactif depuis {} · {origin}", ports.join(" "), tree_idle_txt(&ks)),
                    ram: tree_ram(&ks),
                    count: ks.len(),
                    safe: orphan && idle > 120.0,
                    fix: Fix::KillTree,
                    rule: match &sig {
                        Some(s) => format!("{} | {s}", rp.name),
                        None => rp.name.to_string(),
                    },
                    cmd: rp.cmd.as_deref().map(str::to_string),
                },
                &ks,
                &mut covered,
            );
            continue;
        }
        // 2) orphelin : outil de dev sans parent vivant et sans fenêtre
        if dev && p.parent.is_none() && p.window.is_none() && p.age_secs > 60.0 {
            let ks = tree(p.key);
            if !tree_safe_from_protected(&ks) || tree_has_window(&ks) {
                continue;
            }
            let idle = tree_idle(&ks);
            let sig = signature(p);
            push(
                Ghost {
                    key: p.key,
                    name: p.name.to_string(),
                    kind: Kind::Orphan,
                    reason: format!(
                        "parent disparu{} · inactif depuis {}",
                        sig.as_ref().map(|s| format!(" · {s}")).unwrap_or_default(),
                        tree_idle_txt(&ks)
                    ),
                    ram: tree_ram(&ks),
                    count: ks.len(),
                    safe: idle > 120.0,
                    fix: Fix::KillTree,
                    rule: match &sig {
                        Some(s) => format!("{} | {s}", p.name),
                        None => p.name.to_string(),
                    },
                    cmd: p.cmd.as_deref().map(str::to_string),
                },
                &ks,
                &mut covered,
            );
            continue;
        }
        // 3) démons de build
        let gradle = n == "java.exe" && p.cmd.as_deref().map(|c| c.contains("GradleDaemon") || c.contains("KotlinCompileDaemon")).unwrap_or(false);
        if let Some(desc) = BUILD_DAEMONS.iter().find(|(d, _)| *d == n).map(|(_, d)| *d).or(gradle.then_some("démon Gradle / Kotlin")) {
            let is_node_reuse = n != "msbuild.exe" || p.cmd.as_deref().map(|c| c.contains("nodeReuse:true") || c.contains("/nodemode")).unwrap_or(false);
            if is_node_reuse && p.idle_secs > 300.0 && p.window.is_none() {
                let ks = tree(p.key);
                push(
                    Ghost {
                        key: p.key,
                        name: p.name.to_string(),
                        kind: Kind::BuildDaemon,
                        reason: format!("{desc} · inactif depuis {} · se relance tout seul", idle_text(p)),
                        ram: tree_ram(&ks),
                        count: ks.len(),
                        safe: true,
                        fix: Fix::KillTree,
                        rule: p.name.to_string(),
                        cmd: p.cmd.as_deref().map(str::to_string),
                    },
                    &ks,
                    &mut covered,
                );
                continue;
            }
        }
        // 4) apps connues en fond : on prend la racine de l'app (même nom, en remontant)
        if let Some(&(_, desc, safe)) = BACKGROUND_APPS.iter().find(|(a, _, _)| *a == n) {
            let mut root = p.key;
            while let Some(par) = by_key.get(&root).and_then(|q| q.parent) {
                if by_key.get(&par).map(|pp| lname(pp) == n).unwrap_or(false) {
                    root = par;
                } else {
                    break;
                }
            }
            if covered.contains(&root) {
                continue;
            }
            let ks = tree(root);
            if tree_has_window(&ks) || !tree_safe_from_protected(&ks) {
                covered.extend(ks);
                continue;
            }
            let ram = tree_ram(&ks);
            if ram < 30 << 20 {
                continue;
            }
            let idle = tree_idle(&ks);
            push(
                Ghost {
                    key: root,
                    name: p.name.to_string(),
                    kind: Kind::Background,
                    reason: format!("{desc} · aucune fenêtre · inactif depuis {}", tree_idle_txt(&ks)),
                    ram,
                    count: ks.len(),
                    safe: safe && idle > 60.0,
                    fix: Fix::KillTree,
                    rule: p.name.to_string(),
                    cmd: None,
                },
                &ks,
                &mut covered,
            );
            continue;
        }
    }
    // 5) lourds et inactifs : arbres « racine » de la session, sans fenêtre, > 250 Mo, inactifs 10 min
    for p in &cands {
        if covered.contains(&p.key) || protected.contains(&p.key) {
            continue;
        }
        // racine = parent absent ou parent d'un autre nom
        let is_root = match p.parent.and_then(|k| by_key.get(&k)) {
            None => true,
            Some(pp) => !pp.name.eq_ignore_ascii_case(&p.name),
        };
        if !is_root {
            continue;
        }
        let n = lname(p);
        if PROTECTED.contains(&n.as_str()) || is_ai_agent(p) || white(p) {
            continue;
        }
        let ks = tree(p.key);
        if ks.iter().any(|k| covered.contains(k)) || tree_has_window(&ks) || !tree_safe_from_protected(&ks) {
            continue;
        }
        let ram = tree_ram(&ks);
        let idle = tree_idle(&ks);
        if ram > 250 << 20 && idle > 600.0 {
            push(
                Ghost {
                    key: p.key,
                    name: p.name.to_string(),
                    kind: Kind::HeavyIdle,
                    reason: format!("aucune fenêtre · inactif depuis {} · {} processus", tree_idle_txt(&ks), ks.len()),
                    ram,
                    count: ks.len(),
                    safe: false,
                    fix: Fix::KillTree,
                    rule: p.name.to_string(),
                    cmd: p.cmd.as_deref().map(str::to_string),
                },
                &ks,
                &mut covered,
            );
        }
    }
    ghosts.sort_by(|a, b| {
        let ka = Kind::ORDER.iter().position(|k| *k == a.kind);
        let kb = Kind::ORDER.iter().position(|k| *k == b.kind);
        ka.cmp(&kb).then(b.ram.cmp(&a.ram))
    });
    ghosts
}

/// Tous les descendants (enfants d'abord, racine en dernier) : ordre sûr pour tuer un arbre.
pub fn kill_order(procs: &[Proc], root: Key) -> Vec<Key> {
    let mut children: HashMap<Key, Vec<Key>> = HashMap::new();
    for p in procs {
        if let Some(par) = p.parent {
            children.entry(par).or_default().push(p.key);
        }
    }
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        if let Some(c) = children.get(&out[i]) {
            out.extend(c.iter().copied());
        }
        i += 1;
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    //! Chasseur sur de faux instantanés (aucun processus réel touché).
    use super::*;
    use crate::sampler::{Proc, Snapshot, Totals};
    use crate::sys::Port;
    use std::sync::Arc;

    const SESSION: u32 = 1;
    const MOI: u32 = 9000;

    fn proc(pid: u32, parent: Option<u32>, name: &str) -> Proc {
        Proc {
            key: Key { pid, ctime: pid as i64 * 10 },
            ppid: parent.unwrap_or(0),
            parent: parent.map(|pp| Key { pid: pp, ctime: pp as i64 * 10 }),
            name: Arc::from(name),
            path: None,
            cmd: None,
            session: SESSION,
            threads: 4,
            handles: 10,
            base_prio: 8,
            suspended: false,
            cpu: 0.0,
            cpu_time: 0,
            mem: 300 << 20,
            commit: 0,
            peak_ws: 0,
            ws: 300 << 20,
            disk: 0.0,
            gpu: 0.0,
            gpu_mem: 0,
            window: None,
            ports: Arc::from(Vec::new()),
            idle_secs: 3600.0,
            age_secs: 7200.0,
            idle_min: false,
        }
    }

    fn snap(mut procs: Vec<Proc>) -> Snapshot {
        // Vigie elle-même, lancée par un terminal
        procs.push(proc(MOI - 1, None, "WindowsTerminal.exe"));
        procs.push(proc(MOI, Some(MOI - 1), "vigie.exe"));
        Snapshot { procs, totals: Totals::default(), cost_ms: 0.0, ncpu: 8, parts: String::new(), fullscreen: false }
    }

    fn chasse(s: &Snapshot) -> Vec<Ghost> {
        hunt(s, &Context { session: SESSION, self_pid: MOI, whitelist: &[], docker: &[] })
    }

    fn port(p: u16) -> Arc<[Port]> {
        Arc::from(vec![Port { tcp: true, port: p, local_only: true }])
    }

    #[test]
    fn edge_sans_fenetre_inactif_est_sur() {
        let g = chasse(&snap(vec![proc(100, None, "msedge.exe"), proc(101, Some(100), "msedge.exe")]));
        let e = g.iter().find(|g| g.name == "msedge.exe").expect("Edge en fond doit être proposé");
        assert_eq!(e.kind, Kind::Background);
        assert!(e.safe);
        assert_eq!(e.count, 2);
    }

    #[test]
    fn edge_avec_fenetre_pas_propose() {
        let mut fen = proc(101, Some(100), "msedge.exe");
        fen.window = Some(Arc::from("YouTube — Edge"));
        assert!(chasse(&snap(vec![proc(100, None, "msedge.exe"), fen])).iter().all(|g| g.name != "msedge.exe"));
    }

    #[test]
    fn serveur_de_dev_orphelin_inactif() {
        let mut n = proc(200, None, "node.exe");
        n.ports = port(5173);
        let g = chasse(&snap(vec![n]));
        let s = g.iter().find(|g| g.kind == Kind::DevServer).expect("serveur oublié");
        assert!(s.safe, "orphelin + inactif = sûr");
    }

    #[test]
    fn agent_ia_jamais_propose() {
        let mut n = proc(300, None, "node.exe");
        n.ports = port(4000);
        n.cmd = Some(Arc::from(r#"node C:\Users\x\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js"#));
        let mut c = proc(301, None, "node.exe");
        c.cmd = Some(Arc::from("node copilot-language-server --stdio"));
        assert!(chasse(&snap(vec![n, c])).is_empty());
    }

    #[test]
    fn ancetre_de_vigie_jamais_propose() {
        // un serveur de dev qui a lancé Vigie (ex. via npm run) : intouchable
        let mut s = snap(vec![]);
        let mut n = proc(400, None, "node.exe");
        n.ports = port(3000);
        s.procs.push(n);
        for p in &mut s.procs {
            if p.key.pid == MOI - 1 {
                p.parent = Some(Key { pid: 400, ctime: 4000 });
            }
        }
        assert!(chasse(&s).iter().all(|g| g.key.pid != 400));
    }

    #[test]
    fn parent_protege_pas_propose_en_arbre() {
        // un « lourd et inactif » dont l'arbre contient un processus protégé (explorer.exe) n'est pas proposé
        let racine = proc(500, None, "outil.exe");
        let enfant = proc(501, Some(500), "explorer.exe");
        assert!(chasse(&snap(vec![racine, enfant])).iter().all(|g| g.key.pid != 500));
    }
}
