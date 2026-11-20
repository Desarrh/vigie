<p align="center"><img src="assets/icon.png" width="110" alt="Vigie"></p>

<h1 align="center">Vigie</h1>

<p align="center">Gestionnaire de tâches Windows ultra léger, en Rust, avec un <b>chasseur de RAM fantôme</b> intégré.<br>
Même interface que l'Explorateur (thème crème / bleu nuit, rendu sur le processeur).</p>

## Lancer

```
cargo build --release
target\release\vigie.exe        (ou dist\Vigie.exe)
```

Un seul exe (~5 Mo), sans installation. Une seule fenêtre : relancer Vigie ramène celle qui est ouverte.

## Ce qu'il fait

**Processus** — arbre parent/enfants ou liste à plat, tri par colonne, cellules colorées selon la charge.
Colonnes : statut (App / Fond / Système / Suspendu / 👻 Fantôme), processeur, mémoire (privée, comme le Gestionnaire Windows),
disque, GPU, mémoire GPU, ports, threads, PID. Un nœud replié affiche le total de son arbre.
Volet de détails : graphiques CPU/RAM du processus, chemin, ligne de commande, parent, ports (cliquer ouvre `http://localhost:port`).
Menu clic droit : basculer vers la fenêtre, fermer proprement, terminer, terminer l'arborescence, suspendre, priorité,
ouvrir l'emplacement (dans l'Explorateur s'il est installé dans `C:\Dev\explorateur`), copier, rechercher en ligne.

**Chasseur de RAM** — affiche « X Go récupérables » et classe les coupables :
serveurs de dev oubliés (node, python, cargo… qui écoutent sur un port), orphelins (terminal fermé), démons de build
(VBCSCompiler, MSBuild, Gradle), apps connues sans fenêtre (Edge en pré-chargement, Widgets, Teams, OneDrive, Adobe…),
arbres lourds et inactifs, VM WSL (`wsl --shutdown`), **Docker** : conteneurs inactifs (`docker stop`) et Docker Desktop
sans conteneur actif (quitte l'app et arrête sa VM). Docker n'est interrogé que s'il tourne, toutes les 20 s. Les cibles **sûres** sont pré-cochées ; un clic nettoie.
« Toujours ignorer » ajoute une règle à la liste blanche (`nom.exe` ou `nom.exe | fragment de ligne de commande`).
Jamais proposés : services, Windows, Vigie et ses parents, Claude Code / Claude, VS Code, Cursor, Steam, Discord.

**Journal** (en bas du chasseur) — Go récupérés sur 7 jours, coupable n°1, détail des dernières actions
(`%APPDATA%\Vigie\journal.jsonl`, 90 jours conservés).

**Démarrage** (Ctrl+4) — apps lancées avec Windows (registre Run utilisateur / machine / 32 bits, dossiers Démarrage),
avec la RAM qu'elles occupent en ce moment ; interrupteur pour les désactiver sans rien désinstaller (même mécanisme que
le Gestionnaire des tâches, les entrées « tous les utilisateurs » demandent l'admin).

**Mettre en veille** (clic droit, ou palette) — vide la RAM d'un processus *sans le fermer* : il continue de tourner et
relit ses pages quand il en a besoin.

**Tuer par port** (V1) — dans la palette Ctrl+K, taper `:3000` : la première commande termine ce qui écoute sur ce
port (Entrée), ou Suppr directement. Fini les « EADDRINUSE ».

**Sieste de projet** (V2) — le volet latéral liste les projets en cours sous `C:\Dev\X` (processus dont l'exe ou la ligne
de commande est dans le dossier). Un clic **endort** tout le projet (processus suspendus et RAM libérée, rien n'est fermé),
un autre le **réveille** instantanément. Jamais touchés : processus avec fenêtre (l'éditeur), Claude Code, Vigie et ses parents.
Aussi dans la palette (« Endormir le projet … »).

**Il te manque X Go pour ce jeu** (V3) — Vigie retient le pic de RAM de chaque jeu lancé depuis Steam, Epic, GOG, Riot, EA,
Ubisoft, Xbox… (section « Jeux » du chasseur). Le mode « avant de jouer » affiche pour le dernier jeu « ça passe » ou
« il te manque 1,4 Go » (pic + 10 % de marge − RAM disponible).

**Boîte noire** (V4) — survoler un graphique de Performances (processeur, mémoire, GPU, disque) montre les 5 coupables de cet
instant ; un clic l'épingle sous les graphiques, même si les processus sont morts depuis.

**Performances** — historique CPU, mémoire (engagée, en veille), GPU, disque, réseau ; plus gros consommateurs.

**🎮 Avant de jouer** (Ctrl+G) — chasse les fantômes sûrs, met en veille les apps en arrière-plan
(navigateur, Spotify… ; ni Discord, ni Steam, ni la fenêtre au premier plan), vide la mémoire en veille (admin), passe en
« Performances élevées » sauf si le plan actif est déjà au maximum (Ultimate…). « Quitter » restaure le plan précédent.

## Raccourcis

| | |
|---|---|
| Ctrl+K (ou Ctrl+P) | Palette de commandes floue (« jeu », « admin », nom d'un processus…) |
| Taper / Ctrl+F | Recherche floue (accents, initiales, fautes) : nom, description, PID, titre, ligne de commande, `:3000` = qui écoute sur ce port. Suggestions (↓), Tab complète, historique |
| Ctrl+1 / 2 / 3 / 4 (ou Ctrl+Maj+P / F / G / D) | Processus / Chasseur / Performances / Démarrage — marche en AZERTY |
| ↑ ↓ PgPréc PgSuiv Début Fin (+Maj) | Naviguer (étendre la sélection) ; ← → replier / déplier |
| Entrée | Basculer vers la fenêtre |
| Suppr / Maj+Suppr | Terminer / terminer l'arborescence |
| Espace | Suspendre / reprendre |
| Ctrl+E, Ctrl+C, Ctrl+Maj+C | Emplacement, copier nom+PID, copier le chemin |
| Ctrl+T, F5, F1 | Arbre/liste, mesurer maintenant, aide |

Tant que la souris survole la liste, l'ordre des lignes se fige (on vise sans que ça bouge).

## Ligne de commande (pour le launcher)

```
Vigie.exe --avant-jeu [--json]   sans fenêtre (~1,5 s) : fantômes sûrs, mise en veille des apps en arrière-plan,
                                 cache vidé (si admin), plan d'alimentation ; mémorise les apps endormies
Vigie.exe --apres-jeu [--json]   restaure le plan et réveille les apps endormies (préchargement mémoire) ; ~20 ms
Vigie.exe --etat-dev             1 ligne JSON (format défini avec Ultimate) : serveurs de dev, Docker, WSL ; ~20–30 ms
Vigie.exe --memoire-jeu <exe>   JSON : pic de RAM connu du jeu et ce qu'il manque maintenant (V3)
Vigie.exe --couper-dev [--serveurs] [--docker] [--wsl] [--json]
                                 arrête tout (ou ce qui est précisé), sans UAC ; résumé d'une ligne (JSON avec --json)
```

Exemples :

```
--etat-dev   {"v":1,"items":[{"kind":"node","label":"npm run dev (C:\Dev\Launcher)","pids":[1234,1240],"ram_mo":850,
              "ports":[5173],"projet":"C:\Dev\Launcher"},{"kind":"wsl","label":"WSL (vmmemWSL)","pids":[5724],"ram_mo":993,
              "ports":[],"projet":null}],"total_mo":1843,"ms":7.4}
             kind ∈ node | python | docker | wsl | autre ; liste vide si rien ne tourne
--couper-dev 1 serveur coupé, WSL arrêté, 1,02 Go libérés            (code 0 ; « · sauté : … (admin requis) » si besoin)
--apres-jeu --json  {"message":"Mode jeu terminé · 1 app(s) réveillée(s) (142 Mo préchargés)","libere":0,"reveilles":1}
```
Serveur de dev = outil de dev (node, python, cargo…) de ta session qui écoute sur un port TCP, regroupé avec ses enfants.
Jamais touchés : un arbre avec une fenêtre visible (le jeu, un éditeur), l'appelant et ses parents (Ultimate, même lancé
via `npm run tauri dev`), Ultimate.exe, Claude Code, les processus protégés. Docker est arrêté avec `docker desktop stop`
quand c'est possible, sinon ses processus sont terminés. WSL = `vmmemWSL` seulement (les autres VM Hyper-V ne sont pas touchées).
Le résumé texte est aussi écrit dans `%APPDATA%\Vigie\dernier-jeu.txt`.

## Options (menu ⋯)

Rafraîchissement 0,5 / 1 / 2 / 5 s · GPU par processus on/off · masquer les processus Windows · colonnes ·
confirmation avant de terminer · **Remplacer le Gestionnaire des tâches** (Ctrl+Maj+Échap ouvre Vigie ; clé
`Image File Execution Options\taskmgr.exe`, admin demandé une fois ; décocher rend la main à Windows — ne déplace pas l'exe tant que c'est coché) ·
relancer en administrateur.

Réglages et liste blanche : `%APPDATA%\Vigie\settings.json`. Plantages : `%APPDATA%\Vigie\crash.log`.

## Pourquoi c'est léger

- **Une seule lecture noyau** (`NtQuerySystemInformation`) pour tous les processus : CPU, mémoire, E/S, threads, état suspendu —
  comme System Informer. Chemins et lignes de commande lus une fois par processus et mis en cache.
- **Aucun polling inutile** : un thread mesure à l'intervalle choisi puis dort sur un canal ; l'interface ne redessine qu'à réception
  d'une mesure ou d'une action. Fenêtre réduite → une mesure toutes les 3 s minimum.
- **Rendu CPU** (egui + softbuffer, pas de contexte GPU) : pas de ~150 Mo de pilote, et Vigie ne réveille jamais la carte graphique pendant un jeu.
- GPU / disque / réseau via une seule requête PDH. Ports via IP Helper (~0,5 ms).

Mesuré (release, 16 cœurs, ~370 processus) : **~7 ms par mesure** sur un thread, image ~3–4 ms, **~45–50 Mo** de mémoire privée.
Le coût détaillé s'affiche en survolant « mesure … ms » dans la barre d'état.

## Debug et tests

`VIGIE_CAPTURE=x.png` rend l'interface **entièrement hors écran** (aucune fenêtre, aucun focus, priorité basse) après
quelques mesures, l'enregistre puis quitte. Options : `VIGIE_VIEW=hunter|perf|startup`, `VIGIE_SELECT=nom.exe`,
`VIGIE_PALETTE=1`, `VIGIE_GAME=1`, `VIGIE_BB=1` (boîte noire épinglée), `VIGIE_SIZE=1240x780`, `VIGIE_DUMP=nom.exe` (écrit `x.png.txt`).

`VIGIE_DATA_DIR=dossier` remplace `%APPDATA%\Vigie` (réglages, journal, veille.json) : les tests ne touchent jamais
aux données de l'utilisateur. Règle commune (DIRECTION.md, règle n°0) : un test ne dérange jamais l'utilisateur —
processus jetables, aucune fenêtre visible, aucune entrée simulée, rien sur ses apps ni ses réglages Windows.
