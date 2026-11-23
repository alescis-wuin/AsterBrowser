# Aster — Étape 2 : espaces persistants et organisation des ressources

Statut : **spécification préparatoire, aucune fonctionnalité de l'étape 2 implémentée**.

Base : `main` après fusion de la PR #5, commit `2cd32d471895632b566c8b1d13af7318958d5017`.
Branche dédiée : `feature/aster-workspaces-persistence`.

## 1. Objectif et périmètre

Transformer la liste unique d'onglets actuellement ouverte dans Slint en un organisateur d'espaces de travail persistants. Une ressource sauvegardée **n'est pas** une instance CEF ouverte ; les ressources doivent rester visibles et réouvrables même si leurs onglets sont fermés.

Inclus :
- Créer, renommer, supprimer et réordonner des espaces ; espace par défaut « Navigation » préservé.
- Créer, renommer, déplacer et supprimer des dossiers virtuels imbriqués dans un espace.
- Enregistrer les URLs des onglets, titres, annotations, mots-clés, couleurs et icônes (choix intégré limité et sûr) ; éditer les métadonnées sans recharger le site.
- Épingler et déplacer des entrées, y compris d'un espace vers un autre ; favoris transversaux par référence sans copies de données.
- Basculer entre espaces et ouvrir / activer des onglets réels ; les ressources enregistrées restent présentes après fermeture de leurs onglets.
- Sauvegarder les données en local dans un format versionné et les restaurer au redémarrage, sans chargement massif automatique de pages.
- Ajouter les interactions Slint indispensables et les états vides, sans introduire de dépendance de CEF dans le modèle.

Exclus :
- Deux vues CEF simultanées, explorateur de fichiers, terminal, téléversements, synchronisation réseau, profils Chromium isolés, historique de navigation complet et restauration de formulaires.
- CI/CD, workflows, refonte des moteurs de rendu, migrations de données des profils Chromium.
- Import/export de données non fiables au-delà des contrôles stricts nécessaires à la sauvegarde locale ; fonctionnalité d'import/export optionnelle à traiter séparément.

## 2. Constats vérifiés dans le code

- `browser-contracts::TabId(u64)` et `BrowserSnapshot { tabs, active }` ne décrivent que les sessions actives.
- `browser-core::Browser` gère actuellement `tabs: Vec<Tab>`, `active: Option<TabId>`, `next_id` ; ouvrir un onglet invoque le moteur immédiatement.
- Les commandes `NewTab`, `CloseTab`, `ActivateTab` et les événements d'URL/titre sont déjà disponibles.
- `frontend-slint` affiche uniquement les onglets vivants dans un panneau flottant à survol ; conserver son comportement de superposition et les coordonnées CEF.
- `apps/desktop` relie le moteur, le cœur et le frontend : le raccordement à la persistance y est approprié.

## 3. Modèle de données proposé

**Workspace** : identifiant stable, nom, apparence, ordre, liste de dossiers et d'entrées, identifiant de dernière ressource sélectionnée (optionnel).

**Collection** : identifiant stable, `workspace_id`, `parent_id: Option<CollectionId>`, nom, ordre, apparence. Toute relation parent doit rester dans le même espace et sans cycle.

**SavedResource** : identifiant stable, type `Web` (extensible ultérieurement), URL HTTP(S) ou adresse explicitement autorisée, titre personnalisé ou titre récupéré, notes, tags normalisés, apparence, statut favori.

**WorkspaceEntry** : identifiant stable, `workspace_id`, `resource_id`, `parent_collection_id`, `pinned`, ordre. Une ressource peut être référencée par plusieurs espaces ; les entrées portent l'emplacement et les préférences propres à leur espace.

**LiveTabBinding** : association en mémoire `TabId -> (WorkspaceId, ResourceId?)`. Ne jamais sérialiser `TabId` comme identifiant persistant. L'URL réellement naviguée reste une métadonnée de l'onglet, à synchroniser avec la ressource uniquement selon une règle explicite (ne pas écraser silencieusement une référence sauvegardée lorsque l'utilisateur suit un lien).

**WorkspaceStore** : enveloppe de version de schéma, liste des ressources et espaces, espace actif, préférences simples d'organisation ; identifiants stables et version définie.

Décisions :
- Favoris = propriété dérivée des ressources, pas un espace avec des copies.
- Les dossiers sont virtuels : aucun déplacement sur le disque.
- La suppression d'une entrée ne supprime pas automatiquement une ressource encore référencée ailleurs.
- Supprimer un espace ne doit pas fermer ou perdre silencieusement des onglets ; définir un comportement explicite et testable de transfert ou confirmation.
- À la première ouverture, migrer l'état vide vers un espace « Navigation ». Les onglets existants restent compatibles.
- Les identifiants persistants doivent être distincts du compteur d'onglets en mémoire.

## 4. Architecture cible

Créer un petit module Rust propriétaire du modèle et des opérations d'organisation (idéalement dans `browser-core` ou un sous-module dédié, sans Slint/CEF). Préférer des structures et fonctions simples, des erreurs explicites et des invariants vérifiables, sans généricité superflue.

Étendre les contrats uniquement pour les commandes et instantanés nécessaires à l'UI : `CreateWorkspace`, `RenameWorkspace`, `ActivateWorkspace`, `SaveTab`, `OpenSavedResource`, `MoveEntry`, `SetAppearance` et opérations de dossier. Ne pas surcharger `BrowserCommand` si une interface séparée `WorkspaceCommand` préserve mieux les responsabilités. Garder l'ancien parcours d'onglets fonctionnel.

La couche de stockage doit être injectée depuis `apps/desktop` et testable sur un répertoire temporaire. Pour ce lot, JSON versionné avec écriture temporaire + flush + renommage atomique sur le même volume, ou stockage local existant s'il est découvert ; ne pas introduire SQLite sans nécessité. Prévoir politique de sauvegarde en cas d'erreur et préservation d'une copie récupérable de la dernière version valide. Ne pas journaliser les URL privées ou les notes.

Dans Slint : en-tête Espaces, sélection de l'espace, ressources épinglées, arborescence des dossiers et onglets vivants identifiables, boutons accessibles clavier et souris, menus de création/édition. La barre reste superposée au rendu CEF, avec ouverture 150 ms / fermeture 1000 ms, verrouillage, raccourcis et comportements existants conservés.

## 5. Cycle de vie attendu

1. Au démarrage, charger et valider le magasin versionné avant toute restauration de ressources. En l'absence de fichier, créer un espace par défaut.
2. Ne pas instancier une session CEF par référence enregistrée au démarrage ; n'ouvrir les ressources qu'à la demande. Conserver un seul onglet d'accueil si le comportement historique l'exige.
3. Ouvrir une référence : activer un onglet vivant associé si présent dans cet espace, sinon créer un onglet via le cœur.
4. Fermer un onglet : fermer l'instance CEF et son binding vivant, sans supprimer la référence de l'espace.
5. Changer d'espace : ajuster la sélection et l'onglet visible avec un fallback explicite, sans créer/fermer arbitrairement les onglets des autres espaces.
6. Supprimer/réordonner/déplacer : vérifier existence, propriété, ordre et absence de cycle avant mutation ; les échecs ne laissent pas d'état partiel.
7. Quitter : enregistrer les références, fermer normalement les instances CEF et attendre leurs accusés de fermeture ; aucune sérialisation d'état moteur.

## 6. Sécurité et robustesse

- Valider les URL avant ouverture selon les règles de navigation existantes ; ne jamais accepter `javascript:`, `data:`, `file:` ou des commandes système via des références importées sans mécanisme d'autorisation spécifique.
- Bornes raisonnables : nombre d'espaces, profondeur des dossiers, nombre de ressources, taille du fichier, longueurs de titres/notes/tags.
- Validation des couleurs et des identifiants d'icônes sur liste autorisée ; pas de SVG arbitraire ni de contenu exécutable à cette étape.
- Sur fichier illisible/invalide, ne pas écraser la seule copie valide : signaler l'erreur, conserver la sauvegarde et proposer une récupération contrôlée.
- Ne jamais écrire d'informations de session Chromium, cookies, tokens ni contenus de pages dans la persistance d'espaces.
- Garder la voie mock fonctionnelle sans CEF et sans régression d'entrée clavier/souris.

## 7. Tests et critères d'acceptation

Tests unitaires déterministes pour : création/renommage/suppression ; déplacement entre espaces et dossiers imbriqués ; rejet de cycles, parent étranger et identifiants invalides ; réutilisation multi-espaces ; favoris sans duplication ; séparation ressource/onglet ; fermeture d'onglet sans suppression de référence ; changement d'espace ; sérialisation aller-retour, version inconnue, fichier corrompu et écriture interrompue.

Tests d'intégration mock pour : ouverture d'une ressource à la demande, activation, fermeture, restauration d'un magasin et fonctionnement hors réseau.

Vérifications obligatoires :
```sh
cargo fmt --all -- --check
cargo test -p browser-core -p frontend-slint
cargo check --locked -p aster-desktop
cargo run -p aster-desktop
```
Puis, sur machine CEF :
```sh
cargo check --locked -p aster-desktop --features cef
cargo run -p aster-desktop --features cef
```
Contrôles manuels : sauvegarder plusieurs URL, les placer dans deux espaces, les classer dans des dossiers, fermer tous les onglets, redémarrer, vérifier que les références restent présentes sans création massive de navigateurs, rouvrir une référence, déplacer une ressource, puis vérifier les clics, clavier, redimensionnements et superposition de la barre.

## 8. Livraison

Implémenter cette étape en un lot cohérent et isolé sur `feature/aster-workspaces-persistence`, depuis `2cd32d4`. Documenter l'emplacement du stockage et les décisions de restauration. Ne pas supprimer les validations antérieures. Fournir les commandes de récupération et de lancement, les contrôles réellement exécutés et les limites restantes. Ouvrir une PR vers `main` une fois les tests passés ; ne pas fusionner sans validation native CEF/Wayland de l'utilisateur.

**Cette page prépare l'étape 2 ; elle n'en revendique pas l'implémentation.**
