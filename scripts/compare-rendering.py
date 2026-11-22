#!/usr/bin/env python3
"""Summarize Aster's existing opt-in GPU and Fontconfig telemetry.

Usage: python scripts/compare-rendering.py legacy=legacy.log adaptive=adaptive.log
Requires no third-party packages. Figures are averages of logged windows, not
GPU timer queries; use equal duration, viewport and workload for comparisons.
"""
import argparse
import re
from pathlib import Path
from statistics import mean

KEY_VALUE = re.compile(r"([A-Za-z_][A-Za-z_0-9]*)=([^\s]+)")
GPU_FIELDS = (
    "imported", "importer_full_copy", "presentation_writes",
    "raw_damage_pct", "unique_damage_pct", "overlap_saved_pct",
    "import_wall_avg", "submission_cpu_avg", "plan_avg",
    "sync_wait_avg", "import_saved_estimate",
)
COUNTERS = ("full_regions", "partial_regions", "draws", "lazy_full",
            "import_filter_skips")


def parse_line(line: str) -> dict[str, str]:
    return dict(KEY_VALUE.findall(line))


def parse_log(path: Path) -> tuple[list[dict[str, str]], list[dict[str, str]], dict[str, int]]:
    gpu: list[dict[str, str]] = []
    fonts: list[dict[str, str]] = []
    lifecycle: dict[str, int] = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if "[aster:gpu] imported=" in line:
            gpu.append(parse_line(line))
        elif "[aster:fontconfig]" in line:
            fonts.append(parse_line(line))
        elif "[aster:lifecycle]" in line:
            for event in ("cef_shutdown_begin", "cef_shutdown_complete", "cef_shutdown_timeout",
                          "gpu_renderer_setup", "gpu_renderer_teardown", "slint_window_dropped"):
                if event in line:
                    lifecycle[event] = lifecycle.get(event, 0) + 1
    return gpu, fonts, lifecycle


def numeric(values: list[dict[str, str]], field: str) -> float | None:
    numbers = []
    for entry in values:
        try:
            text = entry[field]
        except KeyError:
            continue
        match = re.match(r"[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?", text)
        if match:
            numbers.append(float(match.group()))
    return mean(numbers) if numbers else None


def total(values: list[dict[str, str]], field: str) -> int:
    numbers = []
    for entry in values:
        try:
            numbers.append(int(entry[field]))
        except (ValueError, KeyError):
            continue
    return sum(numbers)


def display_number(value: float | None) -> str:
    return f"{value:.2f}" if value is not None else "n/a"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", metavar="LABEL=LOGFILE")
    args = parser.parse_args()
    items: list[tuple[str, list[dict[str, str]], list[dict[str, str]], dict[str, int]]] = []
    for spec in args.logs:
        if "=" not in spec:
            parser.error(f"expected LABEL=LOGFILE, got {spec}")
        name, file = spec.split("=", 1)
        path = Path(file)
        if not path.is_file():
            parser.error(f"missing file: {path}")
        gpu, font, lifecycle = parse_log(path)
        if not gpu and not font and not lifecycle:
            parser.error(f"no Aster telemetry in {path}; run with ASTER_PERF=1")
        items.append((name, gpu, font, lifecycle))
    print("GPU telemetry (arithmetic mean of periodic windows; bytes are estimates)")
    print("metric".ljust(25) + " ".join(name.rjust(16) for name, *_ in items))
    print("-" * (25 + 17 * len(items)))
    for field in GPU_FIELDS:
        print(field.ljust(25) + " ".join(display_number(numeric(gpu, field)).rjust(16)
                                              for _, gpu, _, _ in items))
    for field in COUNTERS:
        print(field.ljust(25) + " ".join(str(total(gpu, field)).rjust(16)
                                              for _, gpu, _, _ in items))
    print("\nFontconfig (probe_ms and setup_ms in milliseconds)")
    for name, _, fonts, _ in items:
        print(f"{name}: mode={next((x.get('mode') for x in fonts if 'mode' in x), 'n/a')} "
              f"probe_ok={next((x.get('probe_ok') for x in fonts if 'probe_ok' in x), 'n/a')} "
              f"probe_ms={display_number(numeric(fonts, 'probe_ms'))} "
              f"setup_ms={display_number(numeric(fonts, 'setup_ms'))}")
    print("\nLifecycle events (a complete closure should have a cef_shutdown_complete)")
    for name, _, _, events in items:
        print(f"{name}: {events}")


if __name__ == "__main__":
    main()
