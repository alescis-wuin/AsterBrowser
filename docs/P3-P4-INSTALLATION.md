# Application P3 / P4 sur AsterBrowser

Base requise : `main` + les archives P1 Fontconfig et P2 GPU Adaptive/Triple
appliquées dans `feature/fontconfig-gpu-optimizations`, comme dans les journaux
fournis le 21 novembre 2026. Les modifications de P3/P4 sont incrémentales.

```fish
cd /developpement/seynax/aster-browser
git status --short
# Vérifier que P1/P2 sont sauvegardées avant de créer une branche.
# Si elles ne sont pas encore commitées, les commiter en premier.
git switch -c feature/wayland-event-pump
unzip -o ~/Téléchargements/AsterBrowser-P3-P4-Wayland-EventLoop.zip -d .
cargo fmt --all
cargo test -p browser-contracts
cargo test -p engine-cef --features gpu
cargo test -p frontend-slint
cargo test -p aster-desktop --features cef-gpu
cargo check -p aster-desktop --features cef
cargo check -p aster-desktop --features cef-gpu
```

**Attention :** Git refuse la création d'une branche si le nom existe déjà.
La commande `git switch -c ...` ne met jamais en danger les changements en cours
mais il faut éviter de travailler sur un état non sauvegardé.

Validation P3, sans XWayland :

```fish
env -u DISPLAY ASTER_LIFECYCLE=1 ASTER_PERF=1 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --linux-display wayland --renderer cpu --pump-mode legacy \
  2>&1 | tee /tmp/aster-wayland-cpu.log

env -u DISPLAY ASTER_LIFECYCLE=1 ASTER_PERF=1 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --linux-display wayland --renderer auto --pump-mode legacy \
  2>&1 | tee /tmp/aster-wayland-gpu.log
```

Validation P4, sur la configuration X11 GPU connue comme fonctionnelle :

```fish
env ASTER_LIFECYCLE=1 ASTER_PERF=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive --pump-mode legacy \
  2>&1 | tee /tmp/aster-pump-legacy.log

env ASTER_LIFECYCLE=1 ASTER_PERF=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive --pump-mode event \
  --pump-idle-ms 250 \
  2>&1 | tee /tmp/aster-pump-event.log

python scripts/compare-loop.py \
  legacy=/tmp/aster-pump-legacy.log \
  event=/tmp/aster-pump-event.log
```

Les essais Wayland, le GPU NVIDIA, la latence interactive et la fermeture
nécessitent les validations sur la machine de destination ; ces contrôles
n'ont pas été effectués dans l'environnement de préparation des archives.
L'import DMA-BUF accéléré CEF 156 en natif Wayland reste un point expérimental.
