# Revue EGM de STACK.md — rapport consolidé

Mode : rapport consolidé, étapes 5 à 8. STACK.md est une fiche de stack, sans plan d'implémentation : l'étape 9 ne s'applique pas. Avant la revue, un rapport de dérive (`2026-11-20_DRIFT_REPORT.md`) a été produit et ses écarts corrigés. Rapports par étape : `zoom_out_report.md`, `comprehension_report.md`, `critic_report.md` ; l'étape 8 est résumée ici.

## Actions par priorité

### 🔴 Bloquants (tous traités)
| # | Action | Fait |
|---|---|---|
| B1 | Évaluer les alternatives | §2 « Alternatives considérées » (rendu CPU, egui, `ntdll`, PDH, Docker, IFEO, copie Program Files) |
| B2 | Risques et sécurité, récupération IFEO | §9 « Risques et mitigations » ; procédure `--set-taskmgr off` / `reg delete` (§5) |
| B3 | « Prudence » trop absolue | §1 « Invariants de sûreté » : chaque action et sa confirmation, cas de la ligne de commande |

### 🟡 Majeurs
| # | Action | Fait |
|---|---|---|
| M1 | `--couper-dev` peut tuer ce qui n'a pas été montré | **Code** : `cle` (`pid:ctime`) dans `--etat-dev`, `--couper-dev --seulement`, marqueur `couper-dev-seulement`, test (clé erronée refusée). Ultimate prévenu |
| M2 | Copie IFEO périmée sans signal | **Code** : comparaison au lancement (thread), toast, commande « Mettre à jour la copie » |
| M3 | Pics des jeux ratés entre deux mesures | **Code** : pic de plage de travail tenu par le noyau pour la racine (corrigé à l'étape 8 : `PeakWorkingSetSize`, pas `PeakPagefileUsage`) |
| M4 | Inactivité inconnue au lancement | **Code** : `releve.json` (fermeture + toutes les 5 min) |
| M5 | « Déjà au maximum » par le nom | **Code** : GUID, puis état minimal du processeur (`PowerReadACValueIndex`), puis nom |
| M6 | Objectifs de DIRECTION.md non mesurés | **Code + mesures** : PDH créé après la première mesure → première image en 72–123 ms (avant : 470–870 ms, objectif 300 ms) ; recherche 0,1–0,5 ms (test `recherche_rapide`) ; `tools/mesure.ps1` versionné |
| M7 | Sieste : fenêtre vérifiée sur le processus seul (étape 8) | **Code** : descendants fenêtrés et ancêtres fenêtrés du même projet exclus, test `sieste_respecte_les_fenetres_de_la_famille` |
| M8 | Document désaligné du code et du README | Socle v0.1.5, `--seulement`, `VigieCli`, README (IFEO, mesures, emoji, régime arrière-plan) |
| M9 | Problème, objectifs, non-objectifs ; glossaire (Explorateur, Ultimate, V1–V4, T1–T5, veille, sieste, mode jeu, règle n°0) | §1 et glossaire |
| M10 | Structure : sous-titres, CLI dispersée, fichiers répétés, historique, backlog en double | H3 partout, tableau unique de la CLI, tableau des fichiers, historique retiré (git), backlog renvoyé à QOL.md |
| M11 | Couleurs du socle recopiées | Retirées : renvoi à `socle::theme` |
| M12 | Intouchables « une seule liste » inexact (étape 8) | §1 : tableau de ce que vérifie chaque chemin ; fonction de garde unique notée en dette |

### 🟢 Mineurs
Chiffres corrigés (19 rôles, ~120 Mo de contexte GPU, taille de l'exe), liens Markdown, variables de test en tableau, `\games\` et chemins codés en dur en dette, plan de validation pour chaque « pas testé en conditions réelles », alternative Docker avec condition de réouverture.

## Retenu sans changement (arbitrages)
- Découpage de `qol.rs` et de `ui.rs` / `app.rs` : noté en dette, pas fait (consigne : ne pas découper `ui.rs` maintenant).
- API Docker par tube nommé : écartée pour l'instant, condition de réouverture écrite.

## Hors du dépôt
`C:\Dev\DIRECTION.md` est en retard sur Vigie (thème copié, emoji « à remplacer », AZERTY, Ultimate « non ») : signalé à la session de coordination.

## Traçabilité
| Constat brut | Action |
|---|---|
| Zoom out 1, 2, 3, 4, 5, 6, 7, 8, 9 | M11, M10, M6, M10, M2, M10, M12, M10, M5 |
| Catalyseur 1, 2, 3, 4, 5, 6 | M4, M3, M1, M2, M5, arbitrage Docker |
| Compréhension 1–12 | M9 (glossaire), M10, mineurs |
| Critique (conception) B1, B2, M1–M7 | B1, B2, M8, §5 « qui peut appeler », §9 reprise, §2 Windows, M6, M5, M9 |
| Critique (structure) B1, B2, M1–M7 | B1, B2, M9, M10, M10, M8, M10, M10, M10 |
| Critique (style) B1, M1–M17 | B3, mineurs et M8–M11 |
| Étape 8 : 1, 2, 3, 4, 5–9 | M12, M3/M7, alternative Docker, backlog, mineurs |
