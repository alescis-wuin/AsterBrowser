# Aster Browser — P0 scaffold

Modular Rust browser experiment. **Current status: Slint chrome shell with a deterministic mock engine. It does NOT render web pages yet.**

## Architecture

- `browser-contracts`: platform-neutral state, actions and BrowserEngine trait.
- `browser-core`: tab lifecycle, commands, mock engine, deterministic tests.
- `frontend-slint`: replaceable GUI frontend; no dependency on browser-core or CEF.
- `engine-cef`: reserved CEF integration module, not implemented.
- `aster-desktop`: composition root, currently selects MockEngine.

Future frontends implement the same contracts and depend only on `browser-contracts`.
Keep CEF lifecycle/threading concerns strictly inside the CEF adapter.

## Linux / CachyOS prerequisites

Rust stable (Slint 1.19 requires Rust >=1.95), C++20 toolchain, CMake, Ninja, standard desktop development libraries.

```sh
cargo fmt --all -- --check
cargo test -p browser-core
cargo check -p aster-desktop
cargo run -p aster-desktop
```

## CEF upstream smoke test (separate window)

```sh
./scripts/cef-smoke.sh
```

This builds and launches upstream `cefsimple`, not the Aster UI. It may download large CEF binaries and requires CEF runtime dependencies.
See https://github.com/tauri-apps/cef-rs for platform notes.

## Gate P0

- [x] Repository/workspace initialized.
- [x] Contracts and dependency boundaries implemented.
- [x] Core tab command handling and unit tests written.
- [x] Slint UI wired to mock navigation commands.
- [ ] Compile and run on actual Linux host (not available in authoring environment).
- [ ] Run CEF smoke test (not available in authoring environment).
- [ ] Integrate CEF surface into Slint and validate input, resize, rendering, sandbox and shutdown.

**P0 is not complete until the final three boxes are validated.**

## Security and licensing

Do not disable CEF sandbox in production. Check Slint licensing and CEF redistribution terms before distribution. The mock does not fetch URLs or execute JavaScript.
