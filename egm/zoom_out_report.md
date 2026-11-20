# Étape 5 — Vue d'ensemble (zoom out + catalyseur inventif) sur STACK.md

## Zoom out (verdict : passe, avec réserves)
Vérifié dans le code : 24 tests + 1 ignoré, `BACKGROUND_INTERVAL` = 10 s, Docker toutes les 20 s, pics des jeux toutes les 30 s, `VigieCli`, règles partagées réutilisées, tailles de `ui.rs` et `app.rs`.

1. **Majeur** — §4 recopie les 20 couleurs du socle : la copie qui divergera. → Supprimer le tableau, renvoyer vers `socle::theme` / `explorateur/STACK.md` §4. Ne garder que « Ce qui diffère ».
2. **Majeur** — §9 est un backlog en double de `C:\Dev\QOL.md`, avec 6 lignes déjà faites ; §7 « Possibles », pareil. → Retirer les lignes faites, déplacer les pistes ouvertes dans `QOL.md`, ne garder que la règle commune.
3. **Majeur** — la mesure de §1 n'est pas reproductible (`scratchpad/mesure.ps1` hors dépôt) et le README la contredit (45–50 Mo, « 3 s minimum », emoji). → Versionner le script (`tools/mesure.ps1`), aligner le README.
4. **Majeur** — « Audit ponytail » (§6) est un journal de corrections. → Faire remonter les invariants (ctime vérifié, fenêtre « cloakée » = visible, intouchables partagés) dans les principes ; l'historique va dans git.
5. **Majeur** — la copie IFEO devient périmée sans le dire. → Au lancement, comparer la version de `%ProgramFiles%\Vigie\Vigie.exe` à la sienne et proposer la mise à jour (UAC).
6. Mineur — trop de détail bas niveau (§3 threads, §4 tailles, §2 `.cargo`). → Une phrase par thread ; tailles = écarts avec DIRECTION.md seulement.
7. Mineur — `qol.rs` est rangé selon l'historique ; `ancestors` (règle de sécurité) y est mal placée. → Décrire par rôles ; à terme, `ancestors` à côté de `PROTECTED`.
8. Mineur — répétitions dans §5 (fichiers cités deux fois, plein écran dans « Threads », raccourcis en double avec le README).
9. Mineur — « déjà au maximum » : reconnu d'abord par GUID, puis par mots-clés. Le document laisse croire que seul le nom compte.
- Hors document : `C:\Dev\DIRECTION.md` est en retard (theme copié, emoji de Vigie « à remplacer », AZERTY, socle « recommandé », Ultimate « non »).

## Catalyseur inventif (faux compromis)
1. **Majeur** — inactivité inconnue pendant que Vigie était fermée. → À la fermeture, enregistrer `(pid, ctime) → temps CPU` ; au lancement, même temps CPU = inactif depuis au moins la fermeture (minimum exact, aucun coût quand l'app est fermée).
2. **Majeur** — pics des jeux (V3) ratés entre deux mesures espacées de 10 s. → Lire le pic tenu par le noyau (`PeakPagefileUsage`, déjà dans la classe 5) pour la racine du jeu.
3. **Majeur** — `--couper-dev` refait sa sélection : un serveur apparu après `--etat-dev` est tué sans avoir été montré. → `--etat-dev` renvoie `pid:ctime`, `--couper-dev --seulement <clés>` ne coupe que ces clés (nouveau marqueur `VigieCli`).
4. **Majeur** — la copie IFEO périmée (même constat que zoom out n° 5).
5. Mineur — « déjà au maximum » par le nom. → Lire l'état minimal du processeur (`PROCTHROTTLEMIN` via `PowerReadACValueIndex`).
6. Mineur — `docker.exe` lancé toutes les 20 s. → API du moteur sur `\\.\pipe\docker_engine`.
