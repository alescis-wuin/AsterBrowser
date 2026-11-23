# Aster — Étape 3 : organisation et recherche

Base : `feature/aster-workspaces-persistence` (PR #6). Cette évolution **dépend** de la validation et de la fusion de l'étape 2. Ne pas fusionner le lot 3 directement vers `main` avant l'étape 2.

## Fonctionnalités

- **Onglet vivant → espace/dossier** : menu `⋯` de l'onglet, puis `⇄`. Choisir l'espace et le dossier. L'instance CEF et son `TabId` restent inchangés ; aucun `NewTab`, `CloseTab` ou `Navigate` n'est envoyé pour ce déplacement. L'URL actuelle est enregistrée dans l'espace cible, et l'onglet est rattaché à cet espace. Les références déjà sauvegardées dans l'espace source sont conservées ; il s'agit du déplacement de l'onglet, **pas** de la suppression d'un favori source.
- **Enregistrer / épingler / mettre en favori** un onglet ouvert depuis son menu `⋯` ; les actions épingler et favori enregistrent l'URL au préalable si nécessaire. Une URL non HTTP(S) ou non autorisée par le magasin est refusée, sans fermer l'onglet.
- **Ressources enregistrées** : boutons `☆/★` et `◇/◆` directement sur les lignes de ressources, sans devoir ouvrir l'éditeur. L'éditeur conserve le renommage, les notes, les mots-clés, la couleur et les icônes autorisées.
- **Recherche locale interespaces** : champ dans le panneau, raccourci `Ctrl+Maj+F`, filtres successifs `Tout → Onglets → Ressources → Favoris`. Résultats pour espaces, onglets actifs et ressources enregistrées, même situés dans d'autres espaces. Un clic ou `Entrée` ouvre/active l'élément, en sélectionnant au besoin son espace.
- La recherche porte sur les titres, les URL, les mots-clés et les notes, avec correspondance insensible à la casse et aux accents latins usuels. Tous les termes sont requis et les champs sont pondérés titre > mots-clés > URL > notes. Au maximum 40 résultats affichés. Aucun appel réseau, aucune lecture du contenu d'une page.
- L'état du champ et du filtre de recherche est transitoire : il ne modifie pas les sauvegardes JSON. Les raccourcis et la superposition Slint/CEF de l'étape 1 sont conservés.

## Limites

- Déplacement des onglets via un sélecteur natif Slint ; le **glisser-déposer CEF/natif réel** reste hors périmètre (étape 6).
- La recherche n'indexe pas le contenu des pages ni les fichiers locaux. Il n'y a ni BM25 ni embeddings : le corpus d'informations est la petite base locale des espaces.
- Déplacer un onglet ajoute une référence dans l'espace de destination et conserve les éventuelles références source ; un même objet ressource peut donc être présent dans plusieurs espaces sans duplication de son contenu dans le magasin.
- Favoris et épinglage sont des attributs de ressources sauvegardées ; ils ne restaurent pas automatiquement des instances Chromium après relancement.
- Cette branche ne remplace pas la validation native CEF/Slint de la PR #6.

## Contrôles demandés

Depuis la branche de cette étape, dans un environnement Rust équipé de Slint :

```sh
cargo fmt --all -- --check
cargo test -p browser-core -p frontend-slint -p aster-desktop
cargo check --locked -p aster-desktop
cargo check --locked -p aster-desktop --features cef
cargo run -p aster-desktop --features cef
```

Scénarios manuels :

1. Dans un espace A, ouvrir une URL HTTP(S), utiliser le menu `⋯`, la déplacer vers un dossier d'un espace B, puis revenir à A : vérifier la conservation de l'instance/URL et l'absence de rechargement.
2. Épingler et ajouter aux favoris un onglet non enregistré ; fermer l'onglet, redémarrer ; vérifier la persistance de la référence, de son épingle et du favori.
3. Créer deux ressources dans des espaces distincts avec des notes et tags partagés ; chercher une expression avec accents puis sans accents, alterner les quatre filtres, ouvrir un résultat de B depuis A.
4. Chercher une URL d'un onglet vivant non enregistré ; vérifier qu'il apparaît seulement dans Tout/Onglets.
5. Essayer de déplacer un onglet vers un dossier invalide ou de sauvegarder une URL non autorisée : le stockage ne doit pas être altéré et aucun onglet fermé.
6. Vérifier le clavier, l'accessibilité des résultats, la disparition du panneau au survol, le verrouillage, les clics CEF et le redimensionnement sans décalage de coordonnées.

La fusion doit attendre la compilation, les tests et la validation visuelle sur Linux Wayland/CEF.
