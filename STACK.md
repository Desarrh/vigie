# Vigie — stack technique

## 1. But et direction

Vigie est le gestionnaire de tâches Windows de l'utilisateur. Il remplace le Gestionnaire des tâches de Windows (option Ctrl+Maj+Échap) et intègre un **chasseur de RAM fantôme** : il repère ce qui tourne pour rien (serveurs de dev oubliés, orphelins, apps cachées, Docker/WSL) et propose de le nettoyer, surtout avant de jouer.

Principes :
- **Légèreté mesurée** (socle v0.1.4, fenêtre de test invisible qui dessine vraiment, `scratchpad/mesure.ps1`, 60 s au repos) : **fenêtre ouverte** 34,6 Mo privés, ~1 % d'un cœur (une mesure + une image par seconde) ; **fenêtre réduite ou jeu en plein écran** ~13 Mo, **~0,1 % d'un cœur** (une mesure toutes les 10 s, sans compteurs PDH, aucune image). Avant le socle v0.1.3 : 47,8 Mo (police emoji). Exe ~5,8 Mo.
- **Zéro coût fermé** : pas de service, de tray ni de démarrage auto.
- **Rien n'est redessiné sans raison** : l'interface attend une mesure ou une action.
- **Prudence** : rien n'est tué sans confirmation ou case cochée. Liste de processus protégés. Services et session 0 jamais proposés.
- **Même interface que l'Explorateur.**

## 2. Stack

- **Langage** : Rust, édition 2024 (rustc 1.99). Crate `vigie` v0.1.0.
- **Socle commun** : crate `socle` (tag v0.1.4, `https://github.com/Desarrh/socle`, polices par défaut : ni gras ni emoji ; compilé depuis le tag publié (`.cargo/config.toml` vers `../socle` désactivé : la copie locale contient des modifications en cours)), features `ui`, `settings`, `crash`, `instance`, `testmode` : thème, icônes vectorielles, palette Ctrl+K, autocomplétion floue, toasts, raccourcis AZERTY, instance unique, journal de plantage, mode test. En local, `.cargo/config.toml` (non versionné) le remplace par `../socle`.
- **Interface** : `egui` 0.37.0, `egui-winit` 0.37.0 (clipboard, links), `winit` 0.30.13.
- **Rendu CPU** : `softbuffer` 0.4.8 (présentation GDI), `hdrshot-egui-backend` 0.2.0 (rastériseur logiciel, importé comme `egui_software_backend`), `bytemuck` 1. Il n'y a **aucun contexte GPU**.
- **Données** : `serde` 1 + `serde_json` 1 (réglages, journal, sortie `docker`), `image` 0.25.10 (PNG seulement : icône, captures de debug), `dirs` 7.0.0.
- **API Windows** : `windows-sys` 0.61.2, plus `ntdll` déclaré à la main.
  - `ntdll` : `NtQuerySystemInformation` (classes 5 = processus, 80 = listes mémoire), `NtSetSystemInformation` (purge de la mémoire en veille), `NtQueryInformationProcess` (ligne de commande, classe 60), `NtSuspendProcess` / `NtResumeProcess`.
  - Threading : `OpenProcess`, `TerminateProcess`, `SetPriorityClass`, `QueryFullProcessImageNameW`, `GetSystemTimes`.
  - ProcessStatus : `K32EmptyWorkingSet`, `K32GetProcessMemoryInfo`.
  - IpHelper : `GetExtendedTcpTable` et `GetExtendedUdpTable` (ports).
  - PDH : GPU Engine, GPU Process Memory, GPU Adapter Memory, PhysicalDisk, Network Interface.
  - WindowsAndMessaging + Dwm : fenêtres visibles non masquées, `WM_CLOSE`, premier plan.
  - Power : `PowerGetActiveScheme` / `PowerSetActiveScheme`.
  - Registry : `Image File Execution Options\taskmgr.exe`, clés Run et `StartupApproved`.
  - Shell : `ShellExecuteExW` « runas », `SHGetFileInfoW` (icônes).
  - Version : `GetFileVersionInfoW` (descriptions).
  - Security : jeton, élévation, privilèges.
- **Dépendances externes**, appelées seulement à la demande :
  - `docker.exe`, si Docker Desktop tourne ;
  - `wsl.exe` (`--shutdown`, `--terminate docker-desktop`) ;
  - `powercfg.exe` (nom du plan actif, recréation du plan Performances élevées).
- **Build** : `cargo build --release`, avec un profil release `lto = "thin"`, `codegen-units = 4`, `strip`, `panic = "abort"`. `build.rs` intègre l'icône et les métadonnées via `winresource`.
- **Exe** : `target\release\vigie.exe`, copié à la main dans `dist\Vigie.exe`. `dist\Vigie.ancien.exe` est l'ancienne version, à supprimer.
- **Tests** : `cargo test --release`, 8 tests (ports, actions, mise en veille, démarrage, parsing Docker).

## 3. Architecture

| Fichier | Rôle |
|---|---|
| `main.rs` | Boucle winit + rendu softbuffer, arguments CLI, instance unique, journal de plantage, mode sans fenêtre `--avant-jeu`, captures de debug |
| `app.rs` | État : réglages (`Settings`), historiques, lignes affichées (arbre/plat, tri, ordre figé), actions, nettoyage, mode jeu, journal |
| `ui.rs` | Tout le dessin : barre d'outils, volet latéral, liste virtualisée, détails, Chasseur, Performances, Démarrage, palette, modales, clavier |
| `sampler.rs` | Thread de mesure : instantané de tous les processus (CPU, RAM, disque, GPU, ports, fenêtre, inactivité) |
| `sys.rs` | Couche Windows : lecture noyau, chemins/cmdline, fenêtres, ports, kill/suspendre/priorité/veille, UAC, plan d'alimentation, IFEO, icônes |
| `pdh.rs` | Une requête PDH : GPU par processus et total, mémoire GPU (bornée par l'adaptateur), disque et réseau totaux |
| `hunter.rs` | Règles du chasseur → liste de `Ghost` (catégorie, raison, RAM de l'arbre, « sûr », remède, règle de liste blanche) |
| `docker.rs` | Thread qui interroge `docker ps` / `docker stats` toutes les 20 s, seulement si `com.docker.backend.exe` tourne |
| `startup.rs` | Apps au démarrage : clés Run HKCU/HKLM/WOW64, dossiers Démarrage, activation via `StartupApproved` |
| `cli.rs` | Interface sans fenêtre pour les autres applis (vue `Vue::assembler` injectable pour les tests) : `--etat-dev`, `--couper-dev`, réveil des apps endormies (JSON) |
| `qol.rs` | Pistes validées : V2 projets (`project_of`, `projects`), V3 pics des jeux (`is_game_path`, `track_games`, `missing`), V4 boîte noire (`Moment`, top 5 par ressource), tout calculé sur l'instantané |
| `travail.rs` | Actions longues sur un thread (chasse, mode jeu, fin du mode jeu) à partir d'un plan préparé par l'interface ; état du mode jeu dans `mode-jeu.json` |
| `journal.rs` | Journal des nettoyages en JSONL (append, lecture des N derniers jours, compactage à 90 jours) |
| `winfo.rs` | Description lisible d'un exe (ressource `FileDescription`) |

**Threads**
- **Interface** (winit) : en `ControlFlow::Wait`, il ne redessine que sur un événement, une mesure reçue, ou un `request_repaint` (toast, icône).
- **« echantillonnage »** : boucle mesure → envoi `Arc<Snapshot>` par `mpsc` → réveil de la boucle (`UserEvent::Sample`, pas `request_repaint` : egui ne relaie plus les demandes quand aucune image n'est dessinée). Fenêtre réduite : la mesure est traitée sans dessiner (pics des jeux V3, boîte noire V4, chasseur restent à jour). Il attend ensuite sur `recv_timeout` du canal de commandes (intervalle, GPU, mesure immédiate, arrière-plan). Fenêtre réduite : au moins 10 s entre deux mesures (`BACKGROUND_INTERVAL`), compteurs PDH (GPU, disque, réseau) coupés, ports une mesure sur trois.
- **« travail »** (à la demande) : exécute un plan (`travail::Travail`) — kill d'arbres, `wsl --shutdown`, `docker stop`, purge, plan d'alimentation, réveil — et renvoie un `Fini` (toast + journal). L'interface ne bloque jamais.
- Jeu ou vidéo **en plein écran** (`SHQueryUserNotificationState` : D3D plein écran, occupé, présentation) : même régime que la fenêtre réduite, mesure traitée sans dessin.
- **« icones »** : il reçoit les chemins d'exe et renvoie l'icône RGBA et la description. Chaque chemin n'est chargé qu'une fois.
- **« docker »** : il dort 5 s tant que Docker est arrêté, puis interroge toutes les 20 s.

**Flux de données**
- `Snapshot` → `App::on_snapshot` : historiques (120 points), `hunter::hunt`, puis `rows_dirty`.
- `rebuild_rows()` ne tourne que si quelque chose a changé.
- Les actions de l'interface sont empilées dans `actions: Vec<Action>`, puis appliquées après le dessin (`apply_actions`), comme dans l'Explorateur.

## 4. Direction artistique

Elle vient du **socle** (`socle::theme`, `socle::icons`, `socle::ui`) depuis la migration : plus de copie locale de `theme.rs`. Les 20 couleurs sont celles de l'Explorateur, identiques à avant.

| Rôle | Sombre (défaut) | Clair |
|---|---|---|
| `bg` (fond de fenêtre) | `#0F1B2D` | `#EFE6D3` |
| `chrome` (barre d'outils, barre d'état) | `#0B1525` | `#E4D8BF` |
| `surface` (zone centrale) | `#13223A` | `#F5EDDC` |
| `side` (volet latéral, volet de détails, cartes) | `#101D31` | `#EEE4CF` |
| `code` (champs, fond des graphiques) | `#0D182A` | `#F7F0E1` |
| `border` | `#243756` | `#D6C8AA` |
| `soft` (segmenté, badges, boutons) | `#1C2D48` | `#E8DCC3` |
| `text` | `#E7ECF4` | `#2A2620` |
| `muted` | `#9FB0C8` | `#625846` |
| `accent` (bleu : sélection, CPU, boutons principaux) | `#6FA8FF` | `#2E5E9E` |
| `on_accent` | `#0B1525` | `#FFFFFF` |
| `live` (orange : fantômes, RAM récupérable, GPU, cellules chaudes) | `#F2A65A` | `#A9561A` |
| `add` (vert : mémoire, « sûr », récupéré) | `#7FD49C` | `#2F7346` |
| `del` (rouge : Terminer, disque, erreurs) | `#F0A7A7` | `#A83434` |
| `add_bg` / `del_bg` / `live_bg` | `#123024` / `#341C24` / `#2A2420` | `#E0EBD6` / `#F2DCD4` / `#F1DFC4` |
| `sel_bg` / `hover_bg` | `#23406B` / `#182A44` | `#D5DDE6` / `#EBE1CB` |

**Polices** (chargées depuis `C:\Windows\Fonts`)
- `segoeui.ttf` pour le texte, `consola.ttf` pour le monospace (PID, ports, ligne de commande).
- `segoeuib.ttf` pour la famille « bold ».
- `seguiemj.ttf` (emoji) et `seguisym.ttf` (symboles) en secours.

**Tailles**
- 13,5 : noms dans la liste.
- 12,5 : valeurs.
- 11 : en-têtes de colonnes en MAJUSCULES.
- 11–11,5 : titres de section en MAJUSCULES, gras, couleur `muted` (comme le volet latéral de l'Explorateur).
- 12 : barre d'état.
- 14 : lignes du chasseur et du démarrage.
- 15 : titres des cartes.
- 17 : saisie de la palette.
- 22–28 : grands chiffres (« X Go récupérables »).

**Arrondis**
- Widgets : 6 (`theme::apply`). Menus : 8. Fenêtres : 10.
- Lignes de liste : 6. Éléments du volet latéral : 7. Recherche et lignes du chasseur : 8. Contrôle segmenté : 9 (intérieur transparent).
- Cartes Performances : 10. Carte principale du chasseur et palette : 12. Badges : 8–9.

**Espacements**
- `item_spacing` 8×6 et `button_padding` 10×5, réglés dans `App::new` comme dans l'Explorateur. Différences : délai des info-bulles 0,35 s, et `selectable_labels` n'est pas désactivé (il l'est dans l'Explorateur).
- Lignes de 28 px (`ROW_H`), comme l'Explorateur, avec un retrait horizontal de 6 px pour le fond arrondi.
- Barre d'outils : marges 12×8. Volet latéral : 10×14. Barre d'état : 14×5.
- Vues Chasseur et Démarrage : 24×18. Performances : 20×16.

**Icônes**
- **Icônes vectorielles du socle** (grille 24, trait 1,8, couleur du thème), plus aucun emoji : `ListView`, `Ghost`, `Chart`, `Power` (vues), `Gamepad`, `Broom`, `Shield`, `Sun`/`Moon`, `More`, `Tree`, `Pause`, `ChevronUp/Down/Right`, `Folder`, `Check`, `Refresh` ; catégories du chasseur : `Code`, `Block`, `Settings`, `Eye`, `Sleep`, `Container`, `Terminal`. Boutons via `icons::button` (32 px, `Look::tool` / `Look::active`) et `icons::wide`.
- Les vraies icônes des exe sont extraites via le shell : 16 px dans la liste, 18 px dans le chasseur et le démarrage.
- Logo : `assets/make_icon.py`. Carré crème arrondi, courbe d'activité bleue `#2E5E9E`, point orange `#F28C32`. Il reprend les couleurs de l'icône de l'Explorateur.

**Repris de l'Explorateur à l'identique**
- Palette et `theme::apply`.
- Panneaux `Panel::top/left/bottom` et `CentralPanel`.
- Contrôle segmenté (fond `soft`, segment actif `#2B4470` en sombre ou `surface` en clair).
- Champ de recherche (`code` + bordure, arrondi 8, loupe).
- Bouton thème ☀/🌙 et menu ⋯ à droite.
- En-têtes de colonnes cliquables avec ▲▼.
- Liste virtualisée `show_rows`.
- Contrôle segmenté, palette Ctrl+K, champ de recherche avec autocomplétion et toasts : composants du socle (`socle::ui::{segmented, Palette, autocomplete_field, Toasts}`).
- Modales `Window` centrées (bouton destructif sur `del`).
- Toasts dans la barre d'état, rendu softbuffer, journal de plantage dans `%APPDATA%`.

**Ce qui diffère**
- Pas d'onglets. La barre d'outils est en `chrome`, alors que l'Explorateur a les onglets en `chrome` et la barre d'outils en `surface`.
- Les vues sont dans le contrôle segmenté.
- Cellules colorées selon la charge : `live` × alpha 0,05 à 0,45.
- Graphiques en aire (ligne de 1,5 px + remplissage alpha 0,22).
- Interrupteurs maison dans Démarrage.
- Thème sombre par défaut (`Settings::default`).

## 5. Conventions

**Raccourcis**

| Touche | Action |
|---|---|
| Ctrl+K (Ctrl+P en alias) | Palette de commandes du socle : floue, récents en tête, « Aller à X », « Terminer tous les X » |
| Ctrl+F ou taper | Recherche |
| Ctrl+1 / 2 / 3 / 4 | Vues Processus / Chasseur / Performances / Démarrage — touche physique (`socle::keys`), marche en AZERTY |
| Ctrl+Maj+P / F / G / D | Mêmes vues (alias par lettres) |
| Ctrl+T | Arbre / liste à plat |
| ↑↓ PgPréc PgSuiv Début Fin (+Maj) | Naviguer, étendre la sélection |
| ← → | Replier / déplier |
| Entrée | Basculer vers la fenêtre |
| Suppr / Maj+Suppr | Terminer / terminer l'arbre |
| Espace | Suspendre |
| Ctrl+E | Ouvrir l'emplacement |
| Ctrl+C / Ctrl+Maj+C | Copier nom et PID / copier le chemin |
| Ctrl+A | Tout sélectionner |
| Ctrl+G | Mode jeu |
| F5 | Mesurer maintenant |
| F1 | Aide |
| Échap | Effacer la recherche ou la sélection |

**Fichiers** (`%APPDATA%\Vigie\`)
- `settings.json` : JSON indenté, `#[serde(default)]` donc tolérant aux champs manquants. Écriture atomique (`.tmp` + rename). Sauvegardé à la fermeture et après chaque modification de liste blanche ou de mode jeu.
- `journal.jsonl`, `dernier-jeu.txt`, `crash.log`.

**Code**
- Commentaires et textes d'interface en français, avec une typographie soignée (’ « » · espace insécable fine).
- Doc `//!` en tête de chaque module.
- Pas de `unwrap` sur les appels système : `Option` / `Result<_, String>` avec un message lisible (« accès refusé — relance Vigie en admin… »), affiché en toast rouge.
- Handles Windows fermés par un `Drop`.

**Lancement et UAC**
- Exe portable, sans installation, instance unique du socle (`socle::instance`, mutex + tube nommé) : une 2e instance (Ctrl+Maj+Échap, raccourci) ramène la fenêtre. Ignorée en mode test et juste après une élévation.
- Plantages : `socle::crash` (`%APPDATA%\Vigie\crash.log`), signalés par un toast au lancement suivant.
- Démarre sans admin. « Relancer en admin » fait `ShellExecuteEx runas` avec `--after-elevate`, puis l'instance actuelle se ferme.
- `--set-taskmgr on|off` : instance élevée éphémère qui écrit la clé IFEO.
- `--avant-jeu` / `--apres-jeu` (`--json` en option) : sans fenêtre. `--apres-jeu` n'ouvre ni egui ni thread de mesure (~20 ms).
- `--etat-dev` (1 ligne JSON `{"v":1,"items":[{kind,label,pids,ram_mo,…}],"total_mo"}`, format défini avec Ultimate, ~20–30 ms) et `--couper-dev [--serveurs] [--docker] [--wsl] [--json]` (résumé d'une ligne, sans UAC) : garde-fou dev T2 (`src/cli.rs`, exemples dans le README). Ne touche jamais un arbre avec fenêtre visible, l'appelant et ses ancêtres, Ultimate.exe.
- **Remplacer le Gestionnaire des tâches** : l'instance élevée (`--set-taskmgr on`) copie l'exe dans `%ProgramFiles%\Vigie\Vigie.exe` et inscrit **ce chemin-là** dans IFEO (un exe sous `C:\Dev\…` est modifiable sans admin : l'inscrire ouvrirait une porte). « off » efface la clé et la copie. L'instance non élevée vérifie ensuite la clé et la présence de la copie (détecte une écriture redirigée). Limite : la copie de Program Files ne se met pas à jour toute seule (recocher l'option après une mise à jour).
- **Mode jeu** : l'état (`actif`, plan d'alimentation à restaurer) est dans `%APPDATA%\Vigie\mode-jeu.json`, géré seulement par le mode jeu ; un ancien `prev_power` de settings.json est migré une fois.
- **Virtualisation MSIX** : lancés depuis l'app Claude, nos processus écrivent les nouveaux fichiers d'AppData dans `…\Packages\Claude_…\LocalCache`. Tests et données de test sous `C:\Dev\vigie\target\` ; pour relancer la vraie Vigie : `explorer.exe "C:\Dev\vigie\dist\Vigie.exe"`.
- Ressource de version `VigieCli` = « avant-jeu apres-jeu etat-dev couper-dev memoire-jeu memoire-jeu-dossier » (`build.rs`) : Ultimate y vérifie les commandes comprises avant de les appeler. **Toute nouvelle commande sans fenêtre doit y être ajoutée.**
- `--memoire-jeu <exe | dossier d'installation>` : `{"v":1,"connu":true,"nom","exe","chemin","pic_mo","disponible_mo","manque_mo"}` (V3) ; avec un dossier, le plus gros pic connu sous ce dossier (marqueur `memoire-jeu-dossier`).
- `%APPDATA%\Vigie\veille.json` : apps endormies par `--avant-jeu`, consommé par `--apres-jeu`.

**Debug / tests** (règle n°0 : ne jamais déranger l'utilisateur) : `VIGIE_CAPTURE=x.png` rend l'interface hors écran sans créer de fenêtre (egui + rendu logiciel dans un tampon, priorité basse), options `VIGIE_VIEW`, `VIGIE_SELECT`, `VIGIE_PALETTE`, `VIGIE_GAME`, `VIGIE_BB`, `VIGIE_SIZE`, `VIGIE_DUMP`. `VIGIE_DATA_DIR` redirige réglages, journal et veille.json vers un dossier de test.

## 6. Fonctionnalités

**V1 (vérifiée)**
- Liste des processus en arbre ou à plat, avec un total quand un nœud est replié.
- Colonnes : statut (App / Fond / Système / Suspendu / Fantôme + priorité), CPU, mémoire privée, disque, GPU, mémoire GPU, ports, threads, PID.
- Recherche par nom, description, PID, titre, ligne de commande ou `:port`. Tri, multi-sélection.
- Ordre figé tant que la souris survole la liste.
- Volet de détails : graphiques, chemin, ligne de commande, parent, ports cliquables.
- Actions : terminer, terminer l'arbre, fermer proprement, suspendre, priorité, emplacement (ouvert dans l'Explorateur maison s'il existe), copier, recherche web.
- Chasseur : serveurs de dev, orphelins, démons de build, apps en fond, lourds inactifs, VM WSL. Pré-coche « sûr », liste blanche. Inactivité estimée au lancement (« ≈ »).
- Vue Performances (CPU, mémoire engagée/en veille, GPU, disque, réseau) et top consommateurs.
- Mode « avant de jouer » : ne rétrograde jamais un plan déjà au maximum (l'utilisateur est en Ultimate Performance).
- Palette de commandes Ctrl+K (socle) ; recherche floue avec autocomplétion (historique, noms, descriptions, `:ports`), Tab pour compléter.

**V2 (vérifiée)**
- Vue Démarrage, avec la RAM actuelle de chaque app. L'interrupteur est testé sur une entrée factice.
- Mise en veille sans fermer : testée, 184 Mo libérés sur un processus de test. Aussi dans le mode jeu.
- Journal des nettoyages.

**Étape socle + QoL (vérifiée, tests automatiques et captures hors écran)**
- Migration sur le socle (thème, icônes vectorielles, palette Ctrl+K, autocomplétion floue, toasts, instance unique, crash, Ctrl+1–4 en AZERTY).
- V1 tuer par port (`:3000` puis Entrée ou Suppr dans la palette).
- V2 sieste de projet : test qui endort puis réveille un node jetable ; Vigie et ses parents exclus.
- V3 pics de RAM par jeu : vérifié avec un faux jeu dans un faux `steamapps\common` ; `--memoire-jeu`.
- V4 boîte noire : survol + épinglage sur les graphiques Performances.

**Audit ponytail (corrigé, tests automatiques)**
- Fenêtre sur un autre bureau virtuel (`DWM_CLOAKED_SHELL`) = vraie fenêtre : Edge sur le bureau 2 n'est plus proposé au chasseur ni à `--avant-jeu`.
- Suspendre / sieste : état réel relu juste avant (`sys::suspended_now`), plus de suspensions empilées.
- kill, suspendre, priorité, mise en veille, préchargement : date de création vérifiée (`GetProcessTimes` = `Key::ctime`) contre la réutilisation de PID.
- Découpages de chaînes en minuscules ASCII (plus de panique sur certains caractères).
- `veille.json` fusionné (deux modes jeu de suite sans `--apres-jeu` ne perdent rien).
- Nom du plan d'alimentation par `PowerReadFriendlyName` (plus aucun powercfg, sauf pour recréer un plan masqué).
- Règles partagées : `hunter::is_ai_cmd` (Claude Code, Claude, Copilot, Codex), `hunter::DOCKER_PROCS`, `qol::ancestors` (intouchables) dans le chasseur, la sieste et `--couper-dev`. `dirs` retiré (`%APPDATA%` lu directement).
- Démarrage : table RAM calculée à la mesure ; autocomplétion reconstruite seulement si les processus changent ; minuscules calculées une fois par mesure ; « plus gros consommateurs » = top 5 de la boîte noire.
- Tests : 24 (+1 ignoré) — chasseur sur faux instantanés (Edge sans fenêtre sûr, avec fenêtre non, agent IA non, ancêtre de Vigie non, arbre protégé non), `--couper-dev` (sélection sur vue factice + vrai arrêt limité à un node jetable), mode jeu sur thread avec fusion de veille.json ; les tests qui lancent des processus sont sérialisés (`TEST_PROCESSUS`).

**Pas testé en vrai**
- **Docker** : Docker Desktop était arrêté. Seul le parsing est testé.
- **Remplacement du Gestionnaire des tâches** : demande l'UAC (copie dans Program Files + clé IFEO) ; non exécuté pendant le développement (règle n°0).
- **Vidage de la mémoire en veille** : nécessite l'admin.
- **Mise en veille des vraies apps** de l'utilisateur via le mode jeu.
- **Désactivation d'une vraie entrée** de démarrage.

## 7. Intégrations

- **Explorateur** (en place) : « Ouvrir l'emplacement » lance `C:\Dev\explorateur\dist\Explorateur.exe` (ou `target\release\explorateur.exe`) sur le dossier de l'exe. Sinon, `explorer /select`.
- **Launcher Ultimate** (T1, T2 — côté Vigie prêt, branchement fait par la session Ultimate) : `--avant-jeu --json` au lancement d'un jeu, `--apres-jeu --json` à sa fermeture (restaure le plan, réveille les apps endormies) ; `--etat-dev` puis `--couper-dev` pour « Tout couper / Garder ». Si Vigie est absente, Ultimate continue sans rien bloquer (règle 4).
- **Possibles** :
  - l'Explorateur affiche « quel processus verrouille ce fichier » via Vigie ;
  - le lecteur ou le client YouTube se mettent en liste blanche du mode jeu ;
  - Vigie ouvre un `.log` ou `crash.log` dans le lecteur universel.

## 8. Limites connues / dette technique

- **Fichiers trop gros** : `ui.rs` (1 726 lignes) et `app.rs` (1 086) gagneraient à être découpés par vue.
- ~~Thème copié dans chaque app~~ : résolu, Vigie utilise le socle.
- **Chasseur** :
  - inactivité mesurée seulement depuis l'ouverture de Vigie (estimation par CPU moyen sur la vie du processus) ;
  - listes de noms codées en dur (`DEV_TOOLS`, `BACKGROUND_APPS`, `PROTECTED`) ;
  - pas de lecture de la cible des raccourcis `.lnk` du dossier Démarrage.
- **Compteurs Windows** : mémoire GPU par processus parfois absurde (Windows renvoie p. ex. 597 Go pour `discord_clips`), simplement bornée par le total de l'adaptateur. Le GPU via PDH coûte ~2,6 ms par mesure.
- **Pas de réseau par processus** (ETW, admin) ni de services Windows.
- **Clé IFEO** : elle pointe vers le chemin de l'exe. Déplacer `Vigie.exe` en laissant l'option cochée casse Ctrl+Maj+Échap.
- **Dépendances CLI** : Docker dépend de `docker.exe` dans le PATH. Le nom du plan d'alimentation est lu via `powercfg` (parsing de texte localisé : « Ultimate », « optimal », « élevées »).
- **Pas de mise à jour automatique** (l'Explorateur en a une) ni de dépôt git.
- **Tests** : `--avant-jeu` et `--couper-dev` agissent sur les vraies apps ; ne les tester qu'avec des cibles factices et `VIGIE_DATA_DIR`.

## 9. Pistes QoL (non implémentées, à valider)

Règle commune : **rien ne doit ralentir l'ouverture ni l'image.** Tout réutilise l'instantané déjà mesuré chaque seconde (aucun nouvel appel système en boucle). Ce qui coûte un appel Windows n'est lancé qu'à la demande, pour un seul processus, ou sur le thread de mesure.

Types : **[inédite]** = rien de tel dans les gestionnaires connus. **[existe]** = vu dans Process Explorer, System Informer, btop, Process Lasso, Resource Monitor…, mais absent de Vigie.

Le tableau est classé par utilité pour l'utilisateur.

| # | Piste | Type | Pour | Pourquoi ça reste instantané et léger | Effort |
|---|---|---|---|---|---|
| 1 | ✅ fait (T4, socle) — **Autocomplétion de la recherche** : suggestion grisée en fin de saisie (nom d'exe, description, port), Tab pour accepter, ↑↓ pour l'historique des recherches | existe (navigateurs, btop filtre) | tous | Recherche par préfixe sur ~350 noms déjà en mémoire, < 0,1 ms par frappe. Historique de 20 entrées dans `settings.json` | S |
| 2 | ✅ fait (V2) — **Sieste de projet** : les processus sont regroupés par projet de dev, déduit du chemin dans la ligne de commande (`C:\Dev\vigie` → « vigie : 4 processus, 1,3 Go »). « Endormir le projet » suspend et met en veille tout l'arbre ; « Réveiller » le relance instantanément, sans rien relancer | inédite | dev | Lignes de commande déjà en cache. Regroupement par préfixe de chemin, une fois par mesure. Suspendre et mettre en veille sont des appels noyau de quelques µs par processus | M |
| 3 | ✅ fait (V3) — **« Il te manque 3 Go pour ce jeu »** : Vigie retient le pic de RAM de chaque jeu (exe vu par le launcher ou en plein écran). Avant le lancement suivant, il calcule ce qui manque et propose exactement quoi libérer | inédite | jeu | Un nombre par jeu dans `settings.json`. Comparaison avec la RAM libre déjà mesurée : aucun coût à l'exécution | M |
| 4 | ✅ fait (V4) — **Boîte noire « pourquoi ça a ramé ? »** : clic sur un pic du graphique Performances → les 5 processus responsables à cet instant précis, même s'ils sont morts depuis | inédite (Resource Monitor ne garde rien) | jeu, YouTube | Anneau de 120 × top-5 (PID, nom, CPU, disque) rempli pendant la mesure : ~10 Ko, aucun calcul à l'affichage | S |
| 5 | ✅ fait (V1) — **Tuer par port** : taper `:3000` puis Suppr dans la palette ou la recherche termine ce qui écoute dessus (fini les EADDRINUSE) | inédite sous cette forme | dev | Table des ports déjà dans l'instantané ; une seule action | S |
| 6 | **Naissances / morts surlignées** : vert 2 s pour un nouveau processus, rouge 2 s pour un processus qui disparaît (la ligne reste affichée, barrée) | existe (Process Explorer, System Informer) | tous | Différence de deux ensembles de clés déjà calculée à chaque mesure ; fondu géré par egui, sans mesure supplémentaire | S |
| 7 | **Viseur « quelle appli est-ce ? »** : glisser une cible sur n'importe quelle fenêtre → sélection de son processus (utile pour une pop-up inconnue ou un overlay) | existe (Process Explorer) | tous | `WindowFromPoint` + `GetWindowThreadProcessId` au relâchement de la souris : 1 appel | S |
| 8 | **Fantômes qui reviennent** : Vigie repère une app tuée qui réapparaît dans l'heure, et propose de couper sa vraie source (entrée de démarrage, tâche planifiée, service qui la relance) | inédite | jeu, dev | Ensemble des noms tués récemment (déjà dans le journal) comparé aux naissances de l'instantané. Lecture des tâches planifiées seulement quand une résurrection est détectée | M |
| 9 | **Bouclier premier plan** : pendant un jeu ou une vidéo en plein écran, tout processus d'arrière-plan qui dépasse 20 % de CPU passe en priorité basse, puis revient à la normale à la sortie | existe (Process Lasso ProBalance) | jeu, YouTube | Règle évaluée sur l'instantané existant. `SetPriorityClass` seulement à l'entrée et à la sortie, pas à chaque seconde | M |
| 10 | **Instantané « avant / après »** : touche B pour marquer, puis colonne Δ RAM et liste des nouveaux processus (parfait après un `npm install` ou un build) | inédite dans un gestionnaire | dev | Copie de (clé, mémoire) des ~350 processus : ~8 Ko ; Δ calculé à la reconstruction des lignes | S |
| 11 | **Qui verrouille ce fichier ?** : glisser un fichier sur Vigie, ou le demander depuis l'Explorateur, et le processus coupable est sélectionné avec un bouton « Fermer / Terminer » | existe (Process Explorer, PowerToys File Locksmith) | dev | Restart Manager (`RmGetList`) pour ce seul fichier, ~5–20 ms, à la demande. Rien en arrière-plan | S |
| 12 | **Cœurs CPU en mini-barres** dans le volet latéral (16 barres fines) | existe (btop, Gestionnaire des tâches) | jeu | Un seul appel `SystemProcessorPerformanceInformation` ajouté à la mesure existante (~20 µs) | S |
| 13 | ~~Brancher le launcher~~ → **fait côté Vigie** (QOL T1/T2) : `--avant-jeu`, `--apres-jeu` avec réveil, `--etat-dev`, `--couper-dev` | intégration | jeu | — | — |
| 14 | **Débit réseau par processus** (qui télécharge pendant une vidéo ou un match) | existe (Resource Monitor, System Informer) | YouTube, jeu | Session ETW noyau (admin) sur un thread dédié : événements cumulés, lus une fois par mesure. Désactivé sans admin, donc 0 coût | L |
| 15 | **DLL et handles du processus sélectionné** dans un onglet du volet de détails | existe (Process Explorer) | dev | Lus uniquement à l'ouverture de l'onglet, pour un seul processus, sur un thread ; jamais pour toute la liste | M |
