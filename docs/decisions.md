# ADR-0001 — Toolkit-independent boundaries

Decision: the browser core and contracts contain no Slint or CEF types. Frontends receive commands, immutable snapshots and generic image frames. CEF is selected only in the desktop composition root.

Tradeoff: snapshots are copied for simplicity at P0. Incremental updates are a future performance task, not a prerequisite for correctness.

# ADR-0002 — CEF offscreen rendering

Use CEF OSR with an explicit 16ms message-pump timer on the Slint UI thread. A per-tab mailbox retains only the latest BGRA frame; Slint converts it into its own image buffer. Avoid native child windows because they limit portable, fully custom page composition.

Tradeoff: CPU copies are expensive. The performance/accelerated-OSR implementation is deliberately postponed until real profiling is possible.

# ADR-0003 — Browser security

Keep Chromium sandbox enabled; never disable web security or certificate validation. Do not ship a browser before OS-level sandbox setup, OSR input handling, crash recovery and packaging are tested.

# ADR-0004 — Explicit close protocol

Every CEF browser is closed via `BrowserHost::close_browser` and acknowledged by `LifeSpanHandler::on_before_close`. The runtime pumps pending CEF work after the Slint event loop exits and refuses to claim a clean shutdown if acknowledgments do not arrive within 15 seconds.

# ADR-0005 — Replaceable frontends and mock preservation

No dependency on `cef` exists inside `browser-contracts`, `browser-core` or `frontend-slint`. CEF is an opt-in Cargo feature, while the mock path is available without a native Chromium SDK.

# ADR-0006 — Optional WGPU shared-texture presentation

Decision: retain the CPU BGRA incremental renderer as the default and add an explicitly opted-in WGPU renderer behind the same browser-core contracts. Select one or two presentation buffers per tab at startup. The new `browser-render-wgpu` bridge is shared by the CEF adapter and any WGPU-capable frontend, but is not referenced by `browser-contracts` or `browser-core`.

Tradeoff: CEF's upstream accelerated import performs a full GPU-to-GPU copy and waits for its completion. Aster's secondary RGBA8 presentation textures support GPU shader composition limited to dirty rectangles without CPU readback, but this is not zero GPU-copy. Two buffers may improve presentation isolation at the cost of an additional texture and catch-up GPU draws. Fall back to CPU when GPU import fails, and preserve sandboxing. Keep GPU integration optional until actual hybrid-Linux tests are successful.
