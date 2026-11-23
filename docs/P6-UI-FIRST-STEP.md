# Aster — première intégration UI/UX (Slint)

Cette étape transforme le chrome du navigateur, **sans changer le moteur CEF**,
les contrats de navigation ni la machine d'état des onglets. Elle ne crée pas
encore des espaces de travail persistants : « Navigation » est la vue unique
des onglets actuellement ouverts.

## Comportement

- La barre des onglets flotte au-dessus de la page, sans réduire sa surface de
  rendu ni déclencher de changement de taille CEF à son ouverture.
- Survol du bord gauche de la fenêtre sur 24 px : ouverture après 150 ms.
- Après sortie de la zone latérale : masquage différé de 1000 ms ; retour du
  pointeur dans le panneau annule ce délai.
- Un bouton permet de verrouiller/déverrouiller le panneau. `Alt+E` l'ouvre en
  mode verrouillé ou referme un panneau verrouillé. Échap le ferme lorsqu'il
  n'est pas verrouillé.
- Les onglets du modèle réel sont listés verticalement, activables et
  refermables. Le bouton `+` crée un onglet.
- La barre d'adresse et les actions précédent, suivant et actualiser restent
  visibles lorsque la liste des onglets est masquée.
- `Ctrl+L` et `Ctrl+K` sélectionnent l'URL, `Ctrl+T` crée un onglet et
  `Ctrl+W` ferme l'onglet actif. La souris et la navigation clavier des
  boutons restent possibles.
- L'interface utilise des couleurs cohérentes avec la maquette sombre ; les
  propriétés d'accessibilité décrivent les boutons et les onglets.

## Vérifications sur machine de développement

Depuis la racine du dépôt :

```sh
cargo fmt --all -- --check
cargo test -p browser-core -p frontend-slint
cargo check --locked -p aster-desktop
cargo run -p aster-desktop
```

Avec CEF installé et configuré comme décrit dans le README :

```sh
cargo check --locked -p aster-desktop --features cef
cargo run -p aster-desktop --features cef
```

Contrôles manuels : ouvrir/fermer le panneau depuis le bord gauche et au
clavier, déplacer le pointeur entre les éléments sans masquage intempestif,
cliquer sur un onglet, actualiser, naviguer, redimensionner la fenêtre en
mode ouvert/masqué, vérifier que la taille de la page CEF et les coordonnées
souris sont inchangées lorsque seul le panneau apparaît.

## Limites explicites

- Le rendu natif n'a pas été compilé dans l'environnement de préparation de ce
  lot, qui ne dispose pas de Rust/Cargo ni d'accès direct aux dépendances.
- Le panneau ne restaure pas son verrouillage après redémarrage (étape de
  persistance ultérieure).
- Les espaces enregistrés, dossiers virtuels, couleurs par ressource, tags,
  fichiers locaux et terminal **ne sont pas implémentés** ici.
- Le survol de composants natifs et la gestion du focus doivent être vérifiés
  sur les backends Slint X11/Wayland réels.
