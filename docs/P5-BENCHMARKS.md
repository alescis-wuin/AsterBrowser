# P5 — Banc de benchmarks reproductible et automatisé

## Objectif et périmètre

Le banc de mesure est implémenté en Python, **indépendamment des crates CEF et Slint** (`benchmarks/aster_bench/`). Seuls deux adaptateurs optionnels sont ajoutés :

- `crates/benchmark-control` : transport JSONL/TCP sur **127.0.0.1**, avec jeton aléatoire et sans dépendance à CEF/Slint ;
- `apps/desktop/src/benchmark_adapter.rs` et une petite interface de réveil dans `frontend-slint` : dispatch des commandes sur le thread UI, jamais depuis le socket.

Une exécution normale d'Aster **n'ouvre aucun canal de contrôle**. Celui-ci n'est compilé qu'avec la feature Cargo `benchmark-control` et ne démarre qu'avec `ASTER_BENCH_PORT` et `ASTER_BENCH_TOKEN`.

L'accès CDP est également désactivé sans `ASTER_CDP_PORT`. Attention : **CDP n'est pas authentifié**. Les connexions du runner utilisent exclusivement l'interface loopback. Exécuter les essais sur un poste de confiance ; ne pas ouvrir ni rediriger le port sur le réseau.

Le sandbox de Chromium reste activé : `no_sandbox=0`.

## Installation sous CachyOS / Arch Linux

```fish
sudo pacman -S --needed weston xorg-xwayland mesa-utils

cd /developpement/seynax/aster-browser
uv sync --project benchmarks
cargo fmt --all
cargo build --release -p aster-desktop --features cef-gpu,benchmark-control
```

Le runner vérifie la présence de Weston ou de XWayland uniquement lorsque le mode d'affichage demandé en dépend. L'accélération matérielle des compositeurs headless varie selon EGL, GBM, les pilotes et le matériel : elle n'est **pas** garantie par leur seule installation.

Pour les mesures strictement GPU, commencer avec le mode `host-x11`, correspondant au chemin actuellement validé sur NVIDIA. Le runner distingue explicitement ce mode de `host-wayland`, `weston-wayland` et `weston-xwayland`.

## Planifier une campagne sans lancer de navigateur

```fish
uv run --project benchmarks python -m aster_bench plan \
  --suite gpu --displays host-x11 \
  --scenarios idle,animation,scroll,input,navigation,tabs,resize \
  --repetitions 5
```

Suites : `gpu` (legacy/adaptive × 1/2/3 buffers, **6** variantes), `loop` (**6** variantes), `cpu` (1/2 buffers, **2** variantes), `all` (GPU et loop), `cartesian` (**30** variantes pertinentes du croisement P2/P4). Les combinaisons impossibles ou sans intérêt, comme un triple buffering CPU, ne sont pas générées.

Un garde-fou `--max-cases 500` empêche les campagnes accidentellement gigantesques ; l'augmenter explicitement si nécessaire. Le plan est mélangé de façon reproductible avec `--seed` et reste contrebalancé par répétition.

## Exécuter et générer tous les rapports

```fish
uv run --project benchmarks python -m aster_bench run \
  --suite gpu --displays host-x11 \
  --scenarios idle,animation,scroll \
  --repetitions 3 --warmup 10 --duration 30 \
  --driver auto \
  --output /developpement/seynax/aster-bench-results/gpu-p5
```

Par défaut, le runner compile **une fois** `aster-desktop` en release avec `cef-gpu,benchmark-control`. Utiliser `--skip-build` pour réutiliser un binaire compatible déjà construit. Il lance ensuite un **nouveau processus** pour chaque essai, crée un profil CEF éphémère, active les télémétries, pilote les interactions, ferme proprement la fenêtre puis attend `cef_shutdown_complete`.

Pour le rendu CPU natif Wayland :

```fish
uv run --project benchmarks python -m aster_bench run \
  --suite cpu --displays host-wayland --driver internal \
  --scenarios idle,animation,navigation \
  --repetitions 2 --warmup 5 --duration 10
```

Pour Weston headless GL et sa session XWayland interne :

```fish
uv run --project benchmarks python -m aster_bench run \
  --suite gpu --displays weston-xwayland \
  --scenarios idle,animation --repetitions 2 \
  --warmup 5 --duration 10
```

Pour tester la capture GPU Wayland native, remplacer `weston-xwayland` par `weston-wayland`. Le runner refuse de valider un essai GPU en l'absence d'une image CEF accélérée, y compris si Aster se replie silencieusement sur CPU. Il ne répare pas le problème connu d'allocation `frame_sink_video_capturer_impl.cc: OOM?` observé sous CEF 156 avec le pilote NVIDIA testé.

**Remarque :** Weston headless GL assure un affichage virtuel sans sortie physique, mais n'est pas une preuve d'export DMA-BUF CEF ni de synchronisation GPU correcte. Les résultats de deux modes d'affichage différents ne doivent pas être comparés comme s'ils utilisaient le même chemin graphique. Un rendu virtuel peut également ne pas produire exactement la cadence d'un vrai écran.

## Sélection du meilleur canal de pilotage

| Scénario | Canal automatique | Justification |
|---|---|---|
| idle | CDP | Navigation précise et mesure de l'état du document |
| animation | CDP | Contrôle du document et métriques JavaScript |
| scroll | interne | Vrai chemin `BrowserCommand::Scroll` vers CEF |
| input | interne | Vrai chemin `BrowserCommand::Key`/`Pointer` vers CEF |
| navigation | CDP | Navigation du document et chargement confirmé |
| tabs | interne | Vrais onglets gérés par `BrowserCore` |
| resize | interne | Redimensionnement de la fenêtre Slint, suivi du viewport CEF |

Le mode `--driver auto` utilise CDP **et** le canal interne simultanément quand le scénario l'exige ; le canal interne reste utilisé pour les marqueurs, l'état des onglets et la fermeture propre. `--driver internal` permet un test sans CDP ; `--driver cdp` est refusé pour `tabs` et `resize`, car CDP ne contrôle pas les composants natifs correspondants.

Un échec de connexion CDP obligatoire est un **échec explicite** de l'essai, pas un basculement silencieux vers une autre méthode d'injection.

Les pages de test sont servies par HTTP **uniquement sur 127.0.0.1**, avec contenu déterministe (`seed`), sans appels réseau tiers. Les scénarios déclenchent une animation Canvas, un défilement alterné, une frappe, des navigations locales, des onglets ou des changements de taille de la fenêtre.

## Métrologie et validité

- **Processus :** échantillonnage via `psutil` d'Aster et de tous ses descendants Chromium, avec séparation CPU parent/enfants, RSS total, nombre de processus et nombre de processus `--type=gpu-process` ; intervalle de prélèvement 250 ms.
- **CPU total :** pourcentage équivalent à un cœur logique (100 % = un cœur occupé, >100 % possible). L'addition des RSS **double-compte** certaines bibliothèques partagées : elle n'est pas une mesure PSS.
- **CEF / Slint / GPU :** appel/s, ticks/s, images importées/s, copies et écritures MiB/s, changements complets/partiels et latence P95 des échéances.
- **Normalisation :** lorsque les données existent, volume de copies et d'écritures par image importée ; cela ne remplace pas un protocole A/B à charge contrôlée.
- **Latences :** compteurs `input_to_ui` et `gpu_submit_to_ui` de P4. Ces valeurs sont des proxys d'actualisation UI, **pas** une mesure de scanout ou de photon-to-photon.
- **Préchauffage :** les statistiques de télémetrie n'incluent que la portion comprise entre `[aster:benchmark] mark=measure_begin` et `mark=measure_end`. Les mesures du processus sont filtrées sur la même période.
- **Régressions :** comparaison des médianes de répétitions, par scénario/affichage, avec seuil configurable (15 % par défaut). Minimum 2 répétitions valides. Ce sont des **signaux**, pas des preuves de significativité statistique.

Une campagne GPU est déclarée **invalide** si le journal n'atteste pas `first_valid_frame` et `gpu_renderer_setup device_ready=1`, si une bascule CPU intervient, si les marqueurs sont absents ou si la fermeture CEF n'est pas reconnue. Les erreurs `Fontconfig`, l'échec d'allocation de capture, les panics et les délais de fermeture apparaissent aussi dans les anomalies.

Les sorties par campagne sont :

```text
resultats/
  index.html              # rapport autonome, graphiques SVG
  report.json             # données détaillées et signaux de régression
  results.csv             # une ligne par essai
  displays/.../weston.log # initialisation du compositeur (si utilisé)
  0001-.../
    aster.log             # journaux Slint / CEF / GPU / scheduler
    actions.jsonl         # marqueurs, interactions, durées
    process-tree.csv      # CPU/RSS des processus parents et enfants
    case.json             # résultat et anomalies
```

Le rapport peut être consulté hors connexion. L'exécutable est identifié par son SHA-256 ; la version Git, l'état dirty, le noyau, le GPU et les paramètres figurent dans `report.json`.

## Contrôles

```fish
cargo fmt --all -- --check
cargo test -p benchmark-control
cargo test -p aster-desktop --features cef-gpu,benchmark-control
cargo check -p aster-desktop --features cef-gpu,benchmark-control

uv run --project benchmarks python -m unittest discover -s benchmarks/tests -v
uv run --project benchmarks python -m aster_bench plan \
    --suite all --scenarios idle --repetitions 1
```

Un test d'intégration du runner utilise un faux processus Aster, un vrai socket TCP local et un processus enfant factice. Cela vérifie les commandes, la séparation préchauffage/mesure, la collecte de l'arbre des processus et le rejet des faux succès GPU ; **cela ne valide pas** CEF/WGPU réel ni Weston/NVIDIA.

## Limitations connues

1. L'implémentation Wayland GPU native n'est pas corrigée par P5 ; seules ses tentatives et anomalies sont mesurées.
2. Un affichage Weston headless ne reproduit pas exactement un écran physique (compositor, vsync, VRR, scanout).
3. Le protocole CDP Chromium n'offre pas d'authentification ; éviter les machines partagées et limiter l'accès local.
4. Les appels `__asterBench.metrics()` décrivent la page de test, pas les frames effectivement affichées par Slint ni par le moniteur.
5. Les percentiles P4 du journal sont calculés sur des fenêtres ; leur moyenne n'est pas un percentile global. Les tests mesurent des tendances comparatives.
6. Le matériel graphique et l'intégration Rust doivent être testés sur une machine possédant CEF, Rust, Weston et le GPU cible avant intégration finale.
