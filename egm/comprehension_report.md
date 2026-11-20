# Étape 6 — Test de compréhension (STACK.md seul, sans contexte)

**Ce que propose ce document**

Un gestionnaire de tâches Windows en Rust (Vigie) qui remplace celui de Windows et ajoute un « chasseur de RAM fantôme » (serveurs de dev oubliés, orphelins, Docker/WSL…), surtout avant de jouer. Interface egui en rendu 100 % CPU, thème et composants partagés via le « socle ». Objectifs de légèreté chiffrés, conventions, interface en ligne de commande pour l'appli « Ultimate », pistes d'amélioration.

**Comment fonctionne le système existant, d'après ce document**

Un thread mesure tous les processus chaque seconde (10 s en réduit ou plein écran) et envoie l'instantané à l'interface, qui ne redessine que si quelque chose change. Les actions longues partent sur un thread « travail » à partir d'un plan préparé par l'interface ; elles sont journalisées. Des commandes sans fenêtre permettent au launcher Ultimate de piloter Vigie. Ctrl+Maj+Échap peut ouvrir Vigie via une clé IFEO pointant vers une copie dans Program Files.

## Constats
1. **Majeur** — « V1/V2/V3/V4 » a deux sens (étapes de livraison en §6, numéros de pistes QoL ailleurs), sans définition.
2. **Majeur** — T1, T2, T4 jamais définis.
3. **Majeur** — « Ultimate » : utilisé avant d'être présenté, et confondu avec le plan d'alimentation « Ultimate Performance » ; « la session Ultimate » suppose un mode de travail jamais expliqué.
4. **Majeur** — « veille » désigne trois mécanismes (purge de la mémoire en veille du système, mise en veille d'une app = vider sa RAM, apps « endormies » / sieste = suspension + vidage) ; le « réveil » (préchargement) et les catégories du journal ne sont pas définis.
5. **Majeur** — mode jeu (Ctrl+G), « avant de jouer » et `--avant-jeu` : même fonction ou non ? La séquence d'actions n'est décrite nulle part en une phrase.
6. **Majeur** — « règle 4 » citée sans lien ni définition.
7. **Majeur** — « l'Explorateur » (appli maison) sert de référence dès §1 sans être présenté ; on pense à l'Explorateur Windows.
8. Mineur — la méthode de mesure renvoie à un script non décrit (`scratchpad/mesure.ps1`) ; on ne sait pas ce qui est mesuré.
9. Mineur — ligne `.cargo/config.toml` confuse : quelle source du socle est compilée, et quand ?
10. Mineur — « Virtualisation MSIX… lancés depuis l'app Claude » suppose l'environnement de développement connu.
11. Mineur — noms propres non définis : « audit ponytail », « le lecteur », « le client YouTube ».
12. Mineur — critères de « sûr » non expliqués en dehors des tests ; `Look::tool` / `Look::active` cités sans explication.
