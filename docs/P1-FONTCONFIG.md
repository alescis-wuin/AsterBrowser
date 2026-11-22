# Phase 1: Linux Fontconfig diagnostics and repair

## Goal

Investigate and, when appropriate, repair the warning
`Fontconfig error: Cannot load default config file` **before CEF initializes**.
There is no global file editing, no sandbox relaxation, and no change to
fonts or rendering backends. All behavior is opt-in and isolated to the
process environment inherited by CEF children.

The `FONTCONFIG_FILE` override selects a configuration file;
`FONTCONFIG_PATH` selects the configuration directory. A valid user override
is preserved in `auto` mode. If none is valid, `auto` selects a readable
`/etc/fonts/fonts.conf` or `/usr/local/etc/fonts/fonts.conf`. `force` explicitly
selects a system config or a custom readable file. A bad custom path is an
error rather than an implicit fallback. Custom paths are canonicalized.

| Flag / environment | Values | Default | Effect |
| --- | --- | --- | --- |
| `--fontconfig` / `ASTER_FONTCONFIG_MODE` | `off`, `auto`, `force` | `off` | Do nothing, repair when possible, or select a config explicitly |
| `--fontconfig-file` / `ASTER_FONTCONFIG_FILE` | readable file path | none | Used with `auto` or `force` |
| `--fontconfig-probe` / `ASTER_FONTCONFIG_PROBE` | `on`, `off` / `1`, `0` | `off` | Run `fc-match` once before CEF and report the selected family and duration |

CLI overrides the environment. On platforms other than Linux the settings
are not applied. The probe requires the optional `fc-match` executable;
not finding it does **not** prevent CEF startup. Chromium subprocesses inherit
the selected configuration but never run additional probe processes.

## Controlled test on CachyOS / fish

```fish
# Baseline: no repair, but collect font health and timing data.
env ASTER_FONTCONFIG_MODE=off ASTER_FONTCONFIG_PROBE=1 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 1

# Opt-in repair and probe; use XWayland GPU mode already validated on NVIDIA.
env ASTER_FONTCONFIG_MODE=auto ASTER_FONTCONFIG_PROBE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 1

# Select a file explicitly when auto cannot resolve the system's configuration.
env ASTER_FONTCONFIG_MODE=force ASTER_FONTCONFIG_FILE=/etc/fonts/fonts.conf \
  ASTER_FONTCONFIG_PROBE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 1
```

Check the stderr lines `[aster:fontconfig] mode=...` and
`[aster:fontconfig] probe_ok=true ... probe_ms=...` and see whether the
original error stops occurring. Compare baseline vs repaired runs using
`scripts/compare-rendering.py`. `probe_ms` includes a new `fc-match` process,
so it is diagnostic and **not a measure of page text rendering latency**.

If the warning remains despite a successful `fc-match`, the failure is likely
inside Chromium's own Fontconfig environment or initialization and needs
additional CEF subprocess diagnostics. `fc-match` success by itself is not
proof that Chromium's subprocess loaded the same configuration.

## Checks

```sh
cargo test -p aster-desktop --features cef-gpu
cargo check -p aster-desktop --features cef-gpu
```

The environment is changed only during early startup, before Rust code
spawns any CEF/Slint/WGPU threads. The warning is not fatal by itself;
Fontconfig correctness should be assessed by checking actual font fallback,
not by suppressing messages.
