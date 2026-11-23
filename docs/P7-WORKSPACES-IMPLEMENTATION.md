# Étape 2 — Espaces persistants (implémentation candidate)

Branche : `feature/aster-workspaces-persistence`. La PR doit être **validée avec Slint/CEF sous Linux avant fusion**.

## Fonctionnalités

- Espaces : créer avec `+`, sélectionner, renommer (menu `⋯`), personnaliser la couleur et l'icône, réordonner, supprimer avec double confirmation. La suppression réaffecte les références et les onglets au premier espace restant ; le dernier espace est protégé.
- Dossiers virtuels : créer à la racine ou sous un dossier, renommer, déplacer **dans le même espace**, supprimer. Supprimer un dossier remonte ses enfants au dossier parent sans supprimer les références.
- Onglets actifs : la liste affiche uniquement les onglets de l'espace sélectionné, sans fermer ceux des autres. Un espace vide présente une page d'attente sans charger d'URL automatiquement.
- Références : bouton `☆` sous les onglets pour enregistrer l'onglet actif, `+` dans Ressources pour enregistrer une URL explicite HTTP(S). La liste conserve les références lorsque leurs onglets sont fermés. Cliquer une référence ouvre un onglet CEF à la demande ; si elle est déjà ouverte dans cet espace, l'onglet existant est activé.
- Organisation : menus `⋯` des références pour éditer le titre, une note, les mots-clés, l'icône et la couleur, ajouter/retirer un favori, épingler, réordonner et déplacer dans un autre dossier ou espace.
- Favoris : vue dérivée de la propriété des ressources, sans duplication des objets enregistrés.
- Barres latérale et d'adresse : conservent l'ouverture au survol de 150 ms, la fermeture différée de 1000 ms, le verrouillage et les raccourcis de la première étape ; aucune modification de la surface CEF lors de l'ouverture du panneau.

## Données et sécurité

Chemin par défaut : `${XDG_STATE_HOME:-$HOME/.local/state}/aster-browser/workspaces-v1.json` (et `workspaces-v1.json.bak`). Ne stocke ni cookie, ni contenu de page, ni état interne de Chromium. La sauvegarde JSON validée, versionnée (`version = 1`) utilise un fichier temporaire, `sync_all()` et un renommage atomique. Les fichiers de travail sont privés (`0600` sous Unix) et la précédente version est conservée en `.bak`.

Le programme refuse un schéma inconnu, une URL non HTTP(S) (exception : `about:blank`), des identifiants dupliqués, des dossiers cycliques, des références cassées et des charges excessives. Si la sauvegarde principale est corrompue ou illisible, le programme affiche un message et démarre le modèle par défaut **en lecture seule**, sans écraser le fichier existant. Pour récupérer, quitter Aster, sauvegarder le fichier corrompu sous un autre nom, puis examiner la copie `.bak` et la restaurer manuellement uniquement si elle est valide.

Les identifiants de ressources/espaces sont persistants, ceux des onglets `TabId` restent en mémoire seulement. Le changement d'espace ne restaure pas en masse les URL : seul le clic sur une référence ouvre une page. La navigation ultérieure d'un onglet ne remplace pas silencieusement l'URL enregistrée dans une référence existante.

## Installation et tests à exécuter sur la machine Linux

```fish
 git fetch origin
 git switch --track origin/feature/aster-workspaces-persistence
 cargo fmt --all -- --check
 cargo test -p browser-core -p frontend-slint
 cargo test -p aster-desktop
 cargo check --locked -p aster-desktop
 cargo run -p aster-desktop
```

Avec CEF correspondant au README et les variables `CEF_PATH`, `LD_LIBRARY_PATH` configurées :

```fish
 cargo check --locked -p aster-desktop --features cef
 cargo run -p aster-desktop --features cef
```

Vérifications manuelles : créer Studio et Développement, sauvegarder deux URL, ranger l'une dans un sous-dossier, donner un mot-clé, un favori et une couleur, déplacer l'autre vers Développement, fermer tous les onglets, quitter et redémarrer, vérifier que les références persistent et que leurs pages ne sont pas chargées sans clic. Rouvrir une URL, comparer la liste d'onglets des deux espaces et le rendu/coordonnées CEF, tester le verrouillage de la barre et le redimensionnement. Tester les suppressions de dossier/espace avec enfants et un fichier JSON volontairement invalide **sur une copie de profil**, pas sur les données utilisateur.

## Limites et prochaines étapes

- La validation visuelle, `cargo check` et l'exécution CEF natives ne sont pas disponibles dans l'environnement de préparation. La PR ne doit pas être fusionnée avant ces vérifications.
- Les onglets non enregistrés ne sont pas restaurés après un redémarrage ; seules les références explicitement enregistrées persistent.
- Aucun glisser-déposer, import/export, synchronisation multi-machine, explorateur local, terminal ou affichage de deux pages CEF simultanément à ce stade.
- Les dossiers sont virtuels ; ils ne déplacent aucun fichier local. Les mots-clés sont éditables, sans moteur de recherche dédié à cette étape.
- Pour éviter la régression de la boucle d'événements, le moteur Chromium, sa gestion OSR et les événements de souris sur la page n'ont pas été modifiés.
