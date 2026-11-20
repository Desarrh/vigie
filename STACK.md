# Vigie — stack technique

Fiche de stack et d'état de Vigie : ce qu'elle fait, comment elle est construite, ses règles de sûreté, ses limites. Les règles communes aux applis sont dans [DIRECTION.md](../DIRECTION.md). Les pistes d'amélioration sont dans [QOL.md](../QOL.md). L'usage (raccourcis, ligne de commande avec exemples) est dans le [README](README.md).

## Glossaire

| Terme | Sens |
|---|---|
| **Explorateur** | Explorateur de fichiers maison (`C:\Dev\explorateur`), référence de style des applis. Pas l'Explorateur Windows. |
| **Socle** | Bibliothèque commune des applis ([Desarrh/socle](https://github.com/Desarrh/socle)) : thème, icônes, palette Ctrl+K, autocomplétion, instance unique, journal de plantage, mode test. Compilée dans l'exe. |
| **Ultimate** | Launcher de jeux maison (`C:\Dev\Launcher`) qui pilote Vigie en ligne de commande. Rien à voir avec le plan d'alimentation Windows « Ultimate Performance ». |
| **Fantôme** | Processus qui occupe de la RAM sans servir : serveur de dev oublié, orphelin, démon de build inactif, app connue sans fenêtre, VM WSL, Docker inactif. |
| **Sûr** | Fantôme pré-coché : orphelin ou démon conçu pour se relancer, inactif depuis plusieurs minutes, ou app connue sans aucune fenêtre (règles dans `hunter.rs`). |
| **Mise en veille** (d'une app) | Vider sa RAM sans la fermer (`K32EmptyWorkingSet`) : elle continue de tourner et relit ses pages quand elle en a besoin. |
| **Sieste** (d'un projet) | Suspendre (`NtSuspendProcess`) et mettre en veille tous les processus d'un projet `C:\Dev\X`. **Réveil** : reprise (`NtResumeProcess`) puis préchargement de la mémoire (`PrefetchVirtualMemory`). |
| **Purge du cache** | Vider la liste « mémoire en veille » du système (`NtSetSystemInformation`, admin). Sans rapport avec la mise en veille d'une app. |
| **Mode jeu** | Ctrl+G dans l'interface, ou `--avant-jeu` en ligne de commande (même fonction). Il chasse les fantômes sûrs, met en veille les apps en arrière-plan, purge le cache (si admin) et passe en « Performances élevées » si le plan actif n'est pas déjà au maximum. **Fin du mode jeu** (bouton « Quitter » ou `--apres-jeu`) : plan restauré, apps réveillées. |
| **V1 à V4** | Pistes de [QOL.md](../QOL.md) livrées : V1 tuer par port, V2 sieste de projet, V3 pic de RAM par jeu, V4 boîte noire. |
| **T1, T2, T4, T5** | Pistes transverses de QOL.md : T1 mode jeu lancé par Ultimate, T2 garde-fou dev avant de jouer, T4 autocomplétion, T5 palette Ctrl+K. |
| **Règle n°0** | DIRECTION.md §0 : un test ne doit jamais déranger l'utilisateur (pas de fenêtre visible, pas de focus pris, rien sur ses apps ni ses réglages). |

## 1. But et direction

### Problème
Le Gestionnaire des tâches de Windows montre ce qui tourne, mais il ne dit pas ce qui tourne **pour rien**. Or un PC de dev et de jeu accumule des serveurs oubliés, des démons de build, des VM WSL et des apps cachées qui prennent plusieurs Go juste avant de lancer un jeu.

### Objectifs
- Gestionnaire de tâches complet : processus, arbre, CPU, mémoire, disque, GPU, ports, actions.
- **Chasseur de RAM fantôme** : il montre « X Go récupérables » et nettoie en un clic.
- **Mode jeu** : préparer la machine avant une partie, puis la rendre telle qu'elle était.
- Rapidité et légèreté selon DIRECTION.md §2 (mesures en §7).
- Remplacer Ctrl+Maj+Échap (option, voir §5).

### Non-objectifs
- Pas de service, d'icône dans la zone de notification ni de démarrage automatique : rien ne tourne quand Vigie est fermée.
- Pas de gestion des services Windows ni des processus d'autres utilisateurs (session 0 jamais proposée au chasseur).
- Pas de débit réseau par processus (il faudrait ETW et les droits admin).
- Pas de mise à jour automatique pour l'instant.

### Invariants de sûreté
- **Interface** : chaque action qui termine des processus passe par une confirmation, ou par un clic explicite sur « Nettoyer la sélection » après avoir vu la liste cochée. Concrètement :
  - Suppr, Maj+Suppr, « Terminer tous les X » et `:port` puis Suppr dans la palette ouvrent une confirmation (réglable : « Confirmer avant de terminer » est activé par défaut) ;
  - les cases « sûr » sont pré-cochées, mais rien ne part sans le clic sur « Nettoyer ».
- **Ligne de commande** : `--avant-jeu` et `--couper-dev` agissent sans confirmer. C'est l'appelant (Ultimate) qui doit montrer la liste à l'utilisateur. `--couper-dev --seulement <clés>` garantit qu'on ne coupe que ce qui a été montré (§5).
- **Toujours**, avec des règles communes (`PROTECTED`, `hunter::is_ai_cmd` pour Claude Code, Claude, Copilot et Codex, `qol::ancestors` pour Vigie et la chaîne qui l'a lancée) appliquées par trois chemins :

  | Chemin | Ce qui est vérifié |
  |---|---|
  | Chasseur (`hunter::hunt`) | arbre entier : pas de fenêtre visible, aucun processus `PROTECTED`, pas d'agent IA, pas d'ancêtre de Vigie |
  | Sieste (`qol::projects`) | le processus et ses descendants sans fenêtre, aucun ancêtre fenêtré du même projet, pas `PROTECTED`, pas d'agent IA, pas d'ancêtre de Vigie |
  | `--couper-dev` (`cli::Vue::interdit`) | arbre entier : pas de fenêtre, pas `PROTECTED` (sauf la console attachée), pas Ultimate.exe ni Vigie (`JAMAIS`), pas d'ancêtre de l'appelant, pas d'agent IA |
  | Toute action de terminer (`app`, `travail`) | refus des processus vitaux (`CRITICAL` : leur arrêt provoque un écran bleu ou une déconnexion) et de Vigie elle-même |

  « Fenêtre visible » inclut une fenêtre sur un autre bureau virtuel. Une seule fonction de garde commune à ces chemins est notée en dette (§10).
- **Avant chaque action sur un PID** (terminer, suspendre, priorité, mise en veille, préchargement) : la date de création est revérifiée (`GetProcessTimes` comparée à `Key::ctime`). Un PID réutilisé par un autre processus est refusé.
- Suspendre et endormir relisent l'état réel juste avant d'agir (`sys::suspended_now`) : les suspensions ne s'empilent pas.

## 2. Stack

- **Langage** : Rust, édition 2024 (rustc 1.99). Crate `vigie` 0.1.0.
- **Socle** :
  - version : tag `v0.1.6`, features `ui`, `settings`, `crash`, `instance`, `testmode`, sans la feature `images` (incompatible avec `panic = "abort"`) ;
  - polices : options par défaut du socle (Segoe UI, Consolas, Segoe UI Symbol ; ni gras ni emoji) ;
  - surcharge locale : `.cargo/config.toml` (non versionné) remplace le tag par `../socle`, mais seulement quand la copie locale a la même version que celle verrouillée dans `Cargo.lock`.
- **Interface** : `egui` 0.37, `egui-winit` 0.37 (clipboard, links), `winit` 0.30.13.
- **Rendu CPU** : `softbuffer` 0.4.8 (présentation GDI), `hdrshot-egui-backend` 0.2.0 (rastériseur logiciel, importé comme `egui_software_backend`), `bytemuck`. Aucun contexte GPU.
- **Données** : `serde` + `serde_json` (réglages, journal, sortie Docker), `image` 0.25 (PNG seulement : icône, captures de test). `%APPDATA%` est lu directement.
- **API Windows** : `windows-sys` 0.61.2 (liste des features dans `Cargo.toml`), plus `ntdll` déclaré à la main. Principaux appels :
  - **ntdll** : `NtQuerySystemInformation` (classe 5 = processus et pics noyau, classe 80 = listes mémoire, admin), `NtSetSystemInformation` (purge du cache), `NtQueryInformationProcess` (ligne de commande, classe 60), `NtSuspendProcess` / `NtResumeProcess` ;
  - **Threading** : `OpenProcess`, `GetProcessTimes`, `TerminateProcess`, `SetPriorityClass`, `QueryFullProcessImageNameW`, `GetSystemTimes` ;
  - **Memory / ProcessStatus** : `VirtualQueryEx`, `PrefetchVirtualMemory`, `K32EmptyWorkingSet`, `K32GetProcessMemoryInfo` ;
  - **IpHelper** : `GetExtendedTcpTable`, `GetExtendedUdpTable` (ports) ;
  - **PDH** : GPU Engine, GPU Process Memory, GPU Adapter Memory, PhysicalDisk, Network Interface ;
  - **WindowsAndMessaging + Dwm** : fenêtres visibles (`DWMWA_CLOAKED` : une fenêtre sur un autre bureau virtuel compte comme visible), `WM_CLOSE`, premier plan ;
  - **Power** : `PowerGetActiveScheme`, `PowerSetActiveScheme`, `PowerReadFriendlyName`, `PowerReadACValueIndex` (état minimal du processeur) ;
  - **Registry** : IFEO `taskmgr.exe`, clés Run et `StartupApproved` ; `ExpandEnvironmentStringsW` ;
  - **Shell** : `ShellExecuteExW` « runas », `SHGetFileInfoW` (icônes), `SHQueryUserNotificationState` (plein écran) ;
  - **Storage** : `GetFileVersionInfoW` (descriptions) ;
  - **Security** : jeton, élévation, privilèges.
- **Programmes externes**, seulement à la demande : `docker.exe` (si Docker Desktop tourne), `wsl.exe` (`--shutdown`, `--terminate docker-desktop`), `powercfg.exe` (seulement pour recréer un plan « Performances élevées » masqué).
- **Windows** : développé et utilisé sur Windows 11 (build 26100). Les appels `ntdll` non documentés existent depuis Windows 8.1, mais ils ne sont validés que sur cette machine. En cas d'échec, Vigie se dégrade sans planter : colonne vide, mémoire en veille masquée, toast d'erreur.
- **Build** : `cargo build --release` en priorité basse. Profil : `lto = "thin"`, `codegen-units = 4`, `strip`, `panic = "abort"`. `build.rs` intègre l'icône, les métadonnées et la ressource `VigieCli` (§5).
- **Exe** : `target\release\vigie.exe`, copié dans `dist\Vigie.exe`. Si l'exe est ouvert, il est renommé `Vigie.ancien-HHMMSS.exe` et supprimé à l'installation suivante, une fois libre.

### Alternatives considérées

| Choix | Alternatives écartées | Raison |
|---|---|---|
| Rendu CPU (softbuffer) | egui sur wgpu ou OpenGL | Un contexte GPU coûte environ 120 Mo de pilote (mesuré sur l'Explorateur : de 160 à 40 Mo sans lui). Vigie ne doit jamais réveiller le GPU pendant un jeu. |
| egui (via le socle) | Win32 natif, Tauri | Même interface et mêmes composants que les autres applis maison. Tauri apporterait WebView2 (plus de 100 Mo). |
| `NtQuerySystemInformation` (une lecture pour tous les processus) | Toolhelp + `OpenProcess` par processus, WMI | Une seule lecture noyau (~3 à 8 ms pour ~370 processus) donne CPU, mémoire, E/S, threads, état suspendu et pics. WMI coûte des dizaines de ms et un service. |
| PDH pour le GPU | D3DKMT (`D3DKMTQueryStatistics`) | PDH donne directement le % par moteur et par PID, comme le Gestionnaire des tâches. D3DKMT est plus fin mais demande d'agréger par adaptateur et par nœud. Coût PDH ~2,6 ms par mesure, coupé en arrière-plan. |
| `docker.exe` toutes les 20 s, seulement si Docker tourne | API du moteur sur `\\.\pipe\docker_engine` | Plus simple à écrire ; un processus toutes les 20 s, seulement quand Docker tourne. À rouvrir si la dépendance au PATH ou ce coût posent problème. |
| IFEO `Debugger` sur `taskmgr.exe` | Raccourci clavier global, hook clavier | Seul moyen pour que Ctrl+Maj+Échap et tous les lancements de `taskmgr.exe` ouvrent Vigie. C'est la méthode de Process Explorer. |
| Copie dans `%ProgramFiles%\Vigie` pour IFEO | Inscrire `dist\Vigie.exe` | `C:\Dev` est modifiable sans droits admin. Inscrire un exe modifiable laisserait n'importe quel programme remplacer ce qui se lance à la place du Gestionnaire. |

## 3. Architecture

| Module | Rôle |
|---|---|
| `main.rs` | Boucle winit + rendu softbuffer, arguments de ligne de commande, instance unique et journal de plantage (socle), mode sans fenêtre, mode test, captures |
| `app.rs` | État et actions : réglages, historiques, lignes affichées (arbre, tri, ordre figé), plans de nettoyage et de mode jeu, journal |
| `ui.rs` | Dessin : barre d'outils, volet latéral, liste virtualisée, détails, vues Chasseur, Performances, Démarrage, palette, modales, clavier |
| `sampler.rs` | Thread de mesure : instantané de tous les processus, détection du plein écran |
| `sys.rs` | Couche Windows : lecture noyau, fenêtres, ports, actions vérifiées par date de création, UAC, plan d'alimentation, IFEO, icônes |
| `pdh.rs` | Compteurs PDH : GPU par processus et total, mémoire GPU (bornée par l'adaptateur), disque et réseau totaux |
| `hunter.rs` | Chasseur : règles des fantômes, listes `PROTECTED` et `DOCKER_PROCS`, règle IA `is_ai_cmd` |
| `qol.rs` | Projets (V2), pics des jeux (V3), boîte noire (V4), chaîne d'ancêtres intouchables (`ancestors`) |
| `travail.rs` | Actions longues sur un thread (chasse, mode jeu, fin du mode jeu) ; état du mode jeu (`mode-jeu.json`) |
| `cli.rs` | Commandes sans fenêtre pour Ultimate : `--etat-dev`, `--couper-dev`, réveil des apps endormies |
| `startup.rs` | Apps au démarrage : clés Run, dossiers Démarrage, activation via `StartupApproved`, RAM actuelle |
| `docker.rs` | Thread Docker : `docker ps` / `docker stats` toutes les 20 s si `com.docker.backend.exe` tourne |
| `journal.rs` | Journal des nettoyages (JSONL) et relevé d'inactivité entre deux sessions |
| `winfo.rs` | Description lisible d'un exe (`FileDescription`) |

### Threads
- **Interface** (winit, `ControlFlow::Wait`) : ne dessine que sur un événement, une mesure reçue ou une demande egui (toast, icône).
- **Mesure** : une mesure par seconde, envoyée par `mpsc`, puis réveil direct de la boucle (`UserEvent::Sample`). Ce n'est pas `request_repaint`, car egui ne relaie plus les demandes de dessin quand aucune image n'est dessinée (fenêtre réduite).
- **Travail** (à la demande) : exécute un plan (`travail::Travail`) et renvoie un résultat (`Fini`) affiché en toast. L'interface ne bloque pas pendant `wsl --shutdown`, `docker stop`, les kills ou le plan d'alimentation.
- **Icônes** : charge une fois chaque icône d'exe et sa description.
- **Docker** : dort 5 s tant que Docker est arrêté, interroge toutes les 20 s sinon.

### Régime arrière-plan
Quand la fenêtre est réduite, ou qu'un jeu ou une vidéo est en plein écran (`SHQueryUserNotificationState`) :
- une mesure toutes les 10 s (`BACKGROUND_INTERVAL`) ;
- pas de compteurs PDH (dernières valeurs gardées) ;
- ports une mesure sur trois ;
- chaque mesure est traitée sans dessiner : pics des jeux, boîte noire et chasseur restent à jour.

Les compteurs PDH sont créés **après** la première mesure (leur initialisation prend plusieurs centaines de ms) : la fenêtre a ses données tout de suite.

### Flux de données
- `Snapshot` → `App::on_snapshot` : historiques (120 points), boîte noire, chasseur, projets, suivi des jeux, candidats d'autocomplétion (si les processus ont changé), table RAM du Démarrage (si la vue est ouverte), relevé d'inactivité (toutes les 5 min).
- Pic d'un jeu (V3) : max(somme de la mémoire résidente privée de l'arbre à l'instant, pic de plage de travail de la racine tenu par le noyau). Deux mesures de mémoire résidente, comparées à la RAM disponible (marge de 10 %).
- `rebuild_rows()` ne tourne que si quelque chose a changé. Les minuscules ne sont calculées qu'une fois par mesure, et seulement si une recherche ou le tri par nom en a besoin.
- Les actions de l'interface sont empilées (`actions`) puis appliquées après le dessin, comme dans l'Explorateur.

### Inactivité
- Mesurée depuis l'ouverture de Vigie.
- Au lancement, elle est estimée :
  - par le relevé de la session précédente (`releve.json` : un même processus avec le même temps CPU est inactif depuis au moins la fermeture) ;
  - sinon par le CPU moyen sur la vie du processus.
- Les estimations sont affichées avec « ≈ ».

## 4. Direction artistique

Même direction artistique que l'Explorateur, fournie par le socle : thème (19 rôles, sombre et clair), icônes vectorielles, composants (contrôle segmenté, palette, champ d'autocomplétion, toasts, modales). Référence des couleurs : `socle::theme` et `explorateur/STACK.md` §4 (non recopiées ici).

Ce qui est propre à Vigie :
- Pas d'onglets. Les quatre vues (Processus, Chasseur, Performances, Démarrage) sont dans le contrôle segmenté, et la barre d'outils est en `chrome`.
- Cellules du tableau colorées selon la charge (`live`, alpha 0,05 à 0,45).
- Graphiques en aire (ligne 1,5 px + remplissage alpha 0,22).
- Interrupteurs personnalisés dans la vue Démarrage.
- Pas de gras réel : la famille `bold` est un alias de Segoe UI (police grasse non chargée, économie de RAM). Les titres de section sont en MAJUSCULES `muted`.
- Icônes des exe extraites par le shell (16 px dans la liste, 18 px dans le chasseur et le démarrage).
- Logo : `assets/make_icon.py` (carré crème, courbe d'activité bleue, point orange).

## 5. Conventions

### Fichiers (`%APPDATA%\Vigie\`, ou `VIGIE_DATA_DIR`)
| Fichier | Contenu |
|---|---|
| `settings.json` | Réglages (serde, champs manquants tolérés), liste blanche, historique de recherche, pics des jeux. Écriture atomique. Sauvegardé à la fermeture, à chaque changement de liste blanche, et pour les pics au plus toutes les 30 s. |
| `mode-jeu.json` | Mode jeu actif et plan d'alimentation à restaurer. Géré seulement par le mode jeu. Un ancien `prev_power` de `settings.json` est migré une fois. |
| `veille.json` | Apps mises en veille par le mode jeu (`pid`, date de création), fusionné d'un mode jeu à l'autre, consommé par la fin du mode jeu. |
| `releve.json` | Temps CPU de chaque processus à la fermeture (et toutes les 5 min), pour l'inactivité au lancement suivant. |
| `journal.jsonl` | Nettoyages : `tué`, `chassé`, `coupé`, `sieste`, `veille`, `cache`. 90 jours. |
| `dernier-jeu.txt`, `crash.log` | Dernier résumé du mode jeu, plantages (socle, signalés par un toast au lancement suivant). |

Les tests écrivent leurs données sous `C:\Dev\vigie\target\` avec `VIGIE_DATA_DIR` (contraintes de l'environnement de développement : [DIRECTION.md](../DIRECTION.md) §0).

### Ligne de commande

| Commande | Effet | Sortie | UAC |
|---|---|---|---|
| `--avant-jeu [--json]` | Mode jeu sans fenêtre (~1,5 s : deux mesures pour connaître l'inactivité) | résumé, ou `{"message","libere","reveilles"}` | non (purge du cache seulement si déjà admin) |
| `--apres-jeu [--json]` | Fin du mode jeu (~20 ms) | idem | non |
| `--etat-dev` | Serveurs de dev, Docker, WSL (~20–30 ms) | `{"v":1,"items":[{kind,label,pids,ram_mo,cle,ports,projet}],"total_mo","ms"}` | non |
| `--couper-dev [--serveurs] [--docker] [--wsl] [--seulement c1,c2] [--json]` | Arrête ce qui est demandé ; avec `--seulement`, uniquement les clés `cle` renvoyées par `--etat-dev` (`pid:ctime` vérifié, `docker`, `wsl`) | résumé d'une ligne, ou JSON ; code 2 en cas d'erreur | non (ce qui demande l'admin est sauté et signalé) |
| `--memoire-jeu <exe ou dossier>` | Pic de RAM connu du jeu (le plus gros exe sous un dossier), manque actuel | `{"v":1,"connu","nom","exe","chemin","pic_mo","disponible_mo","manque_mo"}` | non |
| `--set-taskmgr on\|off` | Instance élevée éphémère : copie et clé IFEO | — | oui |

- **Marqueur `VigieCli`** : `build.rs` est la source de vérité. La ressource de version de l'exe liste les commandes comprises (aujourd'hui `avant-jeu apres-jeu etat-dev couper-dev memoire-jeu memoire-jeu-dossier couper-dev-seulement`). Ultimate la lit avant d'appeler une commande. Toute nouvelle commande y est ajoutée.
- **Qui peut appeler** : tout processus de la session de l'utilisateur, avec ses propres droits. Ces commandes ne donnent aucun droit de plus que ceux qu'il a déjà (terminer ses propres processus). Ce sont les intouchables du §1 qui limitent les dégâts d'un appel non désiré.

### Élévation et Gestionnaire des tâches
- Vigie démarre sans admin. « Relancer en admin » fait `ShellExecuteEx runas` avec `--after-elevate`, puis l'instance actuelle se ferme.
- **Remplacer le Gestionnaire des tâches** (option, **non testée en conditions réelles**, §6) :
  - l'instance élevée copie l'exe dans `%ProgramFiles%\Vigie\Vigie.exe` et inscrit ce chemin dans IFEO ; « off » efface la clé et la copie ;
  - l'instance non élevée vérifie ensuite la clé et la présence de la copie (cela détecte une écriture redirigée) ;
  - au lancement, un thread compare l'exe à la copie. Si elles diffèrent, un toast le signale et la palette propose « Mettre à jour la copie du Gestionnaire » (UAC).
- **Récupération** si Vigie ne démarre plus via Ctrl+Maj+Échap : `Vigie.exe --set-taskmgr off`, ou dans un terminal admin `reg delete "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe" /v Debugger /f`.
- Instance unique (socle, mutex + tube nommé) : une deuxième instance ramène la fenêtre. Ignorée en mode test et juste après une élévation.

### Code
- Commentaires et textes en français. Apostrophe ’, guillemets « », espace fine insécable avant ; : ! ?.
- Doc `//!` en tête de chaque module.
- Pas de `unwrap` sur les appels système : `Result<_, String>` lisible, affiché en toast rouge. Handles fermés par `Drop`.

### Tests et mesures (règle n°0)
- `cargo test --release` : 26 tests et 1 ignoré, celui qui écrit une entrée factice dans `HKCU\…\Run`, désactivé pour ne pas toucher au registre de l'utilisateur. Les tests qui lancent des processus jetables sont sérialisés (`TEST_PROCESSUS`).
- `tools\mesure.ps1 [-reduite]` : RAM et CPU au repos, fenêtre de test invisible.
- `--avant-jeu` et `--couper-dev` agissent sur les vraies apps : on ne les teste qu'avec des cibles factices et `VIGIE_DATA_DIR`.

| Variable / option | Effet |
|---|---|
| `--test-hors-ecran` | Fenêtre réelle mais invisible (opacité 0), sans focus, hors de la barre des tâches, en priorité basse (socle) |
| `VIGIE_TEST_REDUIT=1` | Avec `--test-hors-ecran` : fenêtre réduite sans activation (régime arrière-plan) |
| `VIGIE_DATA_DIR=dossier` | Réglages et fichiers de données dans ce dossier |
| `VIGIE_CAPTURE=x.png` | Rendu de l'interface en mémoire, sans fenêtre, enregistré en PNG |
| `VIGIE_VIEW`, `VIGIE_SELECT`, `VIGIE_PALETTE`, `VIGIE_GAME=1`, `VIGIE_BB=1`, `VIGIE_SIZE=LxH` | Pour la capture : vue, processus sélectionné, palette ouverte (et pré-remplie), fenêtre du mode jeu, boîte noire épinglée, taille |
| `VIGIE_DUMP=nom.exe` | Pour la capture : détail des processus de ce nom et des fantômes dans `x.png.txt` |
| `VIGIE_TRACE_DEMARRAGE=1` | Écrit sur stderr le temps jusqu'à la première image avec des données |

## 6. Fonctionnalités et vérification

| Fonction | Vérification |
|---|---|
| Lot 1 : liste des processus (arbre ou plat, totaux repliés, tri, recherche floue, ordre figé au survol), colonnes, volet de détails, actions, chasseur, vue Performances, mode jeu, palette | Captures hors écran ; tests du chasseur sur faux instantanés (Edge sans fenêtre sûr, avec fenêtre non proposé, agent IA, ancêtre de Vigie, arbre protégé) ; actions sur processus jetables |
| Lot 2 : vue Démarrage, mise en veille, journal | Interrupteur sur entrée factice (test ignoré) ; mise en veille sur processus jetable |
| V1 tuer par port, V2 sieste de projet, V3 pic par jeu, V4 boîte noire | Test sieste/réveil sur un node jetable ; faux jeu dans un faux `steamapps\common` ; captures |
| T2 `--etat-dev` / `--couper-dev` | Sélection sur vue factice ; arrêt réel limité à un node jetable ; clé `pid:ctime` erronée refusée |
| T1 mode jeu sur thread, fusion de `veille.json` | Test sur processus jetables, sans purge ni plan d'alimentation |
| T4 autocomplétion, T5 palette (socle) | Test Ctrl+1–4 en AZERTY simulé dans egui ; recherche chronométrée |

**Pas testé en conditions réelles** (avec ce qu'il faudrait faire) :
- **Docker** : Docker Desktop était arrêté. Il faudrait le démarrer dans une VM avec un conteneur inactif.
- **Remplacer le Gestionnaire des tâches** : UAC et HKLM. À faire sur une VM ou un compte jetable.
- **Purge du cache** : demande l'admin, sur une VM.
- **Mise en veille des vraies apps** de l'utilisateur et **désactivation d'une vraie entrée** de démarrage : seulement avec son accord.

## 7. Mesures

`tools\mesure.ps1`, Windows 11, 16 threads, 48 Go, ~370 processus, socle v0.1.5, 60 s au repos après 15 s de chauffe :

| Régime | RAM privée | Plage de travail | CPU (d'un cœur) |
|---|---|---|---|
| Fenêtre ouverte (une mesure et une image par seconde) | 35 Mo | 54 Mo | 1 à 2,5 % selon la charge de la machine |
| Fenêtre réduite (une mesure toutes les 10 s, aucune image) | 13 Mo | 27 Mo | 0,08 % |
| Plein écran | même régime que réduite, non mesuré séparément | | |

Objectifs de DIRECTION.md §2 :
- **Fenêtre utilisable en moins de 300 ms** : première image avec des données en 72 à 123 ms (316 ms quand la machine est à 100 % et Vigie en priorité basse), mesuré avec `VIGIE_TRACE_DEMARRAGE`.
- **Recherche en moins de 5 ms** : 0,1 à 0,5 ms par frappe sur 400 processus (test `recherche_rapide`).
- **Réponse à une frappe en moins d'une image** : une image coûte 3 à 5 ms à 1240×780 (barre d'état, « image … ms »). Non mesuré en 4K.
- **Mesure** : 7 à 30 ms selon la charge (lecture noyau 3 à 8 ms, PDH ~2,6 ms), sur le thread de mesure.

Exe : ~5,9 Mo. La police emoji, retirée depuis le socle v0.1.3, coûtait 13 Mo.

## 8. Intégrations

- **Explorateur** : « Ouvrir l'emplacement » lance `C:\Dev\explorateur\dist\Explorateur.exe` (ou `target\release\explorateur.exe`) sur le dossier de l'exe. Sinon, `explorer /select`.
- **Ultimate** (T1, T2) : côté Vigie, c'est prêt. Le branchement est fait par la session Ultimate et n'est pas vérifié ici.
  - au lancement d'un jeu : `--avant-jeu --json` ;
  - à sa fermeture : `--apres-jeu --json` ;
  - garde-fou dev : `--etat-dev`, puis `--couper-dev --seulement <clés montrées>` ;
  - « Il te manque X Go » : `--memoire-jeu <dossier du jeu>`.
  - Si Vigie est absente ou ancienne (marqueur `VigieCli`), Ultimate continue sans rien bloquer ([DIRECTION.md](../DIRECTION.md) §2, règle 4 : chaque appli marche seule).

## 9. Risques et mitigations

| Risque | Impact | Mitigation | État |
|---|---|---|---|
| Tuer un processus utile | Perte de données non enregistrées | Confirmation dans l'interface ; intouchables (§1) ; pas d'arbre avec fenêtre ; liste blanche | En place, testé sur faux instantanés |
| PID réutilisé entre la mesure et l'action | Action sur le mauvais processus | Date de création revérifiée avant chaque action | En place, testé |
| `--couper-dev` coupe ce qui n'a pas été montré | Serveur apparu entre-temps tué | `--seulement <clés>` (`pid:ctime`) | En place, à brancher côté Ultimate |
| Identification par nom d'exe (`PROTECTED`, `DEV_TOOLS`) | Faux positif ou usurpation de nom | Intouchables (fenêtre, ancêtres, IA) ; rien de pré-coché hors règles « sûr » ; liste blanche | Accepté (même utilisateur, aucun gain de droits) |
| IFEO : Vigie absente ou plantée | Plus de Gestionnaire des tâches via Ctrl+Maj+Échap | Copie dans Program Files ; vérification après écriture ; contrôle de version au lancement ; procédure de récupération (§5) | Non testé en conditions réelles |
| Copie IFEO périmée | Ancienne version derrière Ctrl+Maj+Échap | Contrôle au lancement + toast + commande de mise à jour | En place |
| Interruption pendant une sieste ou un mode jeu (plantage, fermeture) | Processus restés suspendus, plan non restauré | Au lancement suivant : projets affichés « endormi » avec un bouton Réveiller ; mode jeu restauré depuis `mode-jeu.json` (bouton « Quitter ») ; `--apres-jeu` fonctionne aussi plus tard | En place |
| Plan perso performant rétrogradé | Moins de performances en jeu | « Déjà au maximum » : GUID connus, puis état minimal du processeur à 100 %, puis nom ; plan restauré en sortie | En place (un plan perso dont l'état minimal du processeur est sous 100 % sera changé, puis restauré à la sortie) |

## 10. Limites et dette technique

- `ui.rs` (~2 000 lignes) et `app.rs` (~1 300) gagneraient à être découpés par vue.
- **Chasseur** :
  - listes de noms codées en dur (`DEV_TOOLS`, `BACKGROUND_APPS`, `PROTECTED`) ;
  - le marqueur `\games\` de `is_game_path` peut prendre un exe ordinaire pour un jeu ;
  - pas de lecture de la cible des raccourcis `.lnk` du dossier Démarrage.
- **Compteurs Windows** : la mémoire GPU par processus est parfois aberrante (597 Go pour `discord_clips`) ; elle est bornée par le total de l'adaptateur.
- Docker dépend de `docker.exe` dans le PATH, et un processus est lancé toutes les 20 s quand Docker tourne.
- Chemins codés en dur vers l'Explorateur (`C:\Dev\explorateur`), avec un repli sur `explorer /select`.
- Pas de mise à jour automatique (l'Explorateur en a une). Le dépôt git est local, pas encore publié.

- Les règles d'intouchables sont communes (§1) mais assemblées par trois chemins : une seule fonction de garde (par exemple `hunter::interdit`) éviterait qu'un chemin diverge.

Pistes ouvertes : [QOL.md](../QOL.md) (les propositions propres à Vigie y sont transmises par la session de coordination).
