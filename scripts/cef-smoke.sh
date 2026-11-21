#!/usr/bin/env bash
set -euo pipefail
# Independent upstream CEF smoke test, not the integrated Slint renderer.
# Requires network, large disk space and an installed Rust toolchain.
workdir="${1:-$HOME/.cache/aster-cef-smoke}"
if [[ ! -d "$workdir/.git" ]]; then
    git clone --depth 1 https://github.com/tauri-apps/cef-rs.git "$workdir"
fi
cd "$workdir"
cargo run --bin bundle-cef-app -- cefsimple -o target/bundle --release
printf '\nCEF bundle: %s/target/bundle/cefsimple.exe\n' "$workdir"
"$workdir/target/bundle/cefsimple.exe"
