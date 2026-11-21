# ADR-0001 — Toolkit-independent boundaries

Decision: core and shared contracts contain no Slint or CEF types. A frontend is an adapter with a dispatcher callback and snapshot read model. The concrete engine is selected in the desktop composition root.

Tradeoff: snapshots are cloned for simplicity at P0. Use differential events only if profiling justifies them.

# ADR-0002 — No pretend Chromium implementation

CEF/Slint event loop integration and off-screen rendering require a platform-specific spike. Until a tested integration exists, the desktop clearly runs MockEngine and displays a placeholder. Independent cefsimple is used to verify CEF itself.

# ADR-0003 — Browser security

Retain Chromium sandbox and security updates. Avoid insecure defaults to force CEF embedding. Investigate X11/Wayland input, focus, resize, GPU and close lifecycle as explicit acceptance criteria.
