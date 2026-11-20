# Étape 7 — Critique (conception, structure, style) sur STACK.md

Standard appliqué : feature-spec-standard. Les relecteurs notent que STACK.md est une fiche de stack et d'état, pas une spec. Les points « alternatives » et « risques » restent réels et sont traités.

## Bloquants
- **Alternatives non évaluées** (conception B1, structure B1) : rendu CPU contre GPU, egui, `ntdll` contre API documentées, PDH contre D3DKMT, `docker.exe` contre API sur tube, IFEO contre raccourci, copie dans Program Files. → Ajouter « Alternatives considérées » avec les raisons chiffrées.
- **Risques et sécurité dispersés, sans mitigations** (structure B2), dont **IFEO, point unique de défaillance** : si la copie plante, est supprimée ou bloquée, plus de Gestionnaire des tâches (conception B2). → Section « Risques et mitigations », procédure de récupération documentée.
- **« Prudence » trop absolue** (style B1) : Suppr, tuer par port et les cases pré-cochées doivent être décrits précisément ; l'état du branchement côté Ultimate doit être réel.

## Majeurs
- Le document ne correspond plus au code : socle v0.1.5, `--seulement` et `couper-dev-seulement` non documentés, README faux sur l'IFEO (conception M1, structure M4).
- CLI sans confirmation : pas de modèle de menace (qui peut appeler, pire cas, identification par nom) (conception M2).
- Reprise après interruption (sieste, mode jeu) non documentée (conception M3).
- API `ntdll` sans versions de Windows ni repli documentés (conception M4).
- Mesures : une seule mesure, objectifs de DIRECTION.md (16 ms, 300 ms, 5 ms) non rapportés, plein écran non mesuré, chiffres sans source (conception M5, style M2, M11, M14).
- Plan d'alimentation perso : la garantie « ne rétrograde jamais » est fausse si le nom n'est pas reconnu (conception M6, style M16).
- Pas de problème, d'objectifs ni de non-objectifs explicites (conception M7, structure M1).
- Sous-titres en gras au lieu de H3 ; « Lancement et UAC » fourre-tout ; fichiers décrits deux ou trois fois (structure M2, M3, style M17).
- V1 à V4 à double sens ; T1, T2, T4, « ponytail », « règle 4 » non définis (structure M5, style M6, M7).
- §9 « non implémentées » contient des pistes faites ; dette barrée ; règle « aucun appel en boucle » contredite par deux pistes (structure M6, M7, style M8, M9).
- « Remplace le Gestionnaire » et la description IFEO sont présentés comme des faits alors qu'ils ne sont pas testés (style M1). « 20 couleurs » au lieu de 19 rôles ; « gras » alors qu'aucune police grasse n'est chargée (style M4, M5).

## Mineurs (sélection)
- `\games\` trop large dans `is_game_path` ; chemins `C:\Dev\explorateur` codés en dur ; plan de validation pour chaque « pas testé en conditions réelles » (conception N4, N5, N7).
- Liens Markdown absents, variables de debug sans description, registre familier, nombres exacts de lignes qui vieillissent (structure N1-N8, style N1-N20).
