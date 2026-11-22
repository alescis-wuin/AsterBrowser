"""Deterministic experimental design and legal configuration combinations."""
from __future__ import annotations

from dataclasses import dataclass
import random

SCENARIOS = ("idle", "animation", "scroll", "input", "navigation", "tabs", "resize")
DISPLAYS = ("host-x11", "host-wayland", "weston-wayland", "weston-xwayland")


@dataclass(frozen=True)
class Variant:
    name: str
    buffers: int = 2
    damage: str = "adaptive"
    pump: str = "legacy"
    wake: str = "deadline"
    heartbeat_ms: int = 33
    presentation: str = "poll"
    renderer: str = "gpu"

    def args(self, display: str) -> list[str]:
        if display in ("host-x11", "weston-xwayland"):
            backend = "x11"
        else:
            backend = "wayland"
        return [
            "--renderer", self.renderer,
            "--linux-display", backend,
            "--buffers", str(self.buffers),
            "--gpu-damage", self.damage,
            "--gpu-threshold", "65",
            "--gpu-lazy-init", "on",
            "--gpu-sync", "queue",
            "--gpu-import-filter", "empty",
            "--pump-mode", self.pump,
            "--pump-wake", self.wake,
            "--pump-idle-ms", str(self.heartbeat_ms),
            "--gpu-presentation", self.presentation,
        ]

    def as_dict(self) -> dict:
        return vars(self).copy()


def variants(suite: str) -> list[Variant]:
    if suite not in ("gpu", "loop", "cpu", "all", "cartesian"):
        raise ValueError(f"unknown suite: {suite}")
    gpu = [Variant(name=f"gpu_{damage}_{buffers}", buffers=buffers, damage=damage)
           for buffers in (1, 2, 3) for damage in ("legacy", "adaptive")]
    loop = [
        Variant("loop_legacy"),
        Variant("loop_eager33", pump="event", wake="eager"),
        Variant("loop_deadline16", pump="event", heartbeat_ms=16),
        Variant("loop_deadline33", pump="event"),
        Variant("loop_deadline250", pump="event", heartbeat_ms=250),
        Variant("loop_notify33", pump="event", presentation="notify"),
    ]
    if suite == "cpu":
        return [Variant(name=f"cpu_{buffers}", renderer="cpu", buffers=buffers, damage="legacy")
                for buffers in (1, 2)]
    if suite == "gpu":
        return gpu
    if suite == "loop":
        return loop
    if suite == "all":
        return gpu + loop
    # Full crossed P2/P4 matrix, with meaningless duplicate legacy wake modes
    # removed. Cases are explicit rather than creating undocumented fallbacks.
    return [Variant(name=f"cart_{damage}{buffers}_{pump}{heartbeat}_{presentation}",
                    buffers=buffers, damage=damage, pump=pump,
                    heartbeat_ms=heartbeat, presentation=presentation)
            for damage in ("legacy", "adaptive")
            for buffers in (1, 2, 3)
            for pump, heartbeat, presentation in (
                ("legacy", 33, "poll"),
                ("event", 16, "poll"), ("event", 33, "poll"),
                ("event", 250, "poll"), ("event", 33, "notify"))]


@dataclass(frozen=True)
class Case:
    variant: Variant
    display: str
    scenario: str
    repetition: int
    index: int

    @property
    def name(self) -> str:
        return f"{self.index:04d}-{self.display}-{self.variant.name}-{self.scenario}-r{self.repetition}"


def matrix(suite: str, displays: list[str], scenarios: list[str], repetitions: int,
           seed: int, *, shuffle: bool = True) -> list[Case]:
    if repetitions < 1 or repetitions > 100:
        raise ValueError("repetitions must be between 1 and 100")
    if not displays or not scenarios:
        raise ValueError("at least one display and scenario required")
    unknown = set(displays) - set(DISPLAYS)
    if unknown:
        raise ValueError(f"invalid displays: {sorted(unknown)}")
    unknown = set(scenarios) - set(SCENARIOS)
    if unknown:
        raise ValueError(f"invalid scenarios: {sorted(unknown)}")
    # Randomize across variants within each repetition, never lose pairing.
    rng = random.Random(seed)
    result = []
    for repeat in range(1, repetitions + 1):
        batch = [(v, d, s) for v in variants(suite) for d in displays for s in scenarios]
        if shuffle:
            rng.shuffle(batch)
        for variant, display, scenario in batch:
            result.append(Case(variant, display, scenario, repeat, len(result) + 1))
    return result


def best_driver(scenario: str, preference: str = "auto") -> str:
    if preference not in ("auto", "internal", "cdp"):
        raise ValueError(f"unknown driver: {preference}")
    if preference == "cdp" and scenario in ("tabs", "resize"):
        raise ValueError(f"{scenario}: CDP does not operate Aster's native tabs/window; use internal")
    if preference != "auto":
        return preference
    # CDP navigates the document, but internal dispatch exercises the actual
    # Aster -> BrowserCommand -> CEF input path and Slint window lifecycle.
    return "cdp" if scenario in ("idle", "animation", "navigation") else "internal"
