#!/usr/bin/env python3
"""Compare Aster P3/P4 telemetry. No dependencies and no causal claims.

Usage: python scripts/compare-loop.py legacy=/tmp/aster-timer.log event=/tmp/aster-event.log
"""
import argparse
import re
import statistics
from pathlib import Path

RE_PUMP = re.compile(r"\[aster:pump\].*?checks=([\d.]+)/s calls=([\d.]+)/s scheduled=(\d+) fallback=(\d+) skipped=(\d+) requests=(\d+) pump_avg=([\d.]+)ms pump_max=([\d.]+)ms")
RE_SLINT = re.compile(r"\[aster:slint\].*?ticks=([\d.]+)/s")
RE_SCHEDULER = re.compile(r"\[aster:scheduler\] mode=event wake_requested=(\d+) wake_posted=(\d+) wake_dispatched=(\d+) wake_coalesced=(\d+) ticks_per_sec=([\d.]+)")
RE_PUMP_EXTRA = re.compile(r"wake_avoided=(\d+) wake_issued=(\d+) deadline_late_p50_ms=([\d.]+) deadline_late_p95_ms=([\d.]+) deadline_late_p99_ms=([\d.]+) request_to_pump_p50_ms=([\d.]+) request_to_pump_p95_ms=([\d.]+) request_to_pump_p99_ms=([\d.]+)")
RE_LATENCY = re.compile(r"\[aster:latency\] input_to_ui_samples=(\d+) input_to_ui_p50_ms=([\d.]+) input_to_ui_p95_ms=([\d.]+) input_to_ui_p99_ms=([\d.]+) gpu_submit_to_ui_samples=(\d+) gpu_submit_to_ui_p50_ms=([\d.]+) gpu_submit_to_ui_p95_ms=([\d.]+) gpu_submit_to_ui_p99_ms=([\d.]+)")
RE_CPU = re.compile(r"\[aster:load\] process_cpu_pct=([\d.]+) process_cpu_ms=([\d.]+) window_ms=([\d.]+)")
RE_DISPLAY = re.compile(r"\[aster:display\] requested=(\w+) cef_ozone=(\w+) wayland_socket=(\w+) display=(\w+)")
FIELDS = [
    "pump_checks_per_s", "pump_calls_per_s", "slint_ticks_per_s",
    "scheduled_pumps_per_window", "fallback_pumps_per_window", "pump_avg_ms", "pump_max_ms",
    "wake_requested_per_window", "wake_posted_per_window", "wake_dispatched_per_window",
    "wake_coalesced_per_window", "wake_avoided_per_window", "wake_issued_per_window",
    "deadline_late_p50_ms", "deadline_late_p95_ms", "deadline_late_p99_ms",
    "request_to_pump_p95_ms", "process_cpu_pct",
    "input_to_ui_p95_ms", "gpu_submit_to_ui_p95_ms",
]


def read_metrics(path: Path) -> dict:
    if not path.is_file():
        raise ValueError(f"fichier introuvable : {path}")
    metrics = {key: [] for key in FIELDS}
    out = {"closing": False, "first_gpu_frame": False, "gpu_fallback": False, "wayland": "non renseigné", "windows": 0}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if m := RE_PUMP.search(line):
            checks, calls, scheduled, fallback, _, _, avg, high = map(float, m.groups())
            for key, value in {"pump_checks_per_s": checks, "pump_calls_per_s": calls, "scheduled_pumps_per_window": scheduled, "fallback_pumps_per_window": fallback, "pump_avg_ms": avg, "pump_max_ms": high}.items():
                metrics[key].append(value)
            out["windows"] += 1
        if m := RE_PUMP_EXTRA.search(line):
            avoided, issued, p50, p95, p99, _r50, r95, _r99 = map(float, m.groups())
            metrics["wake_avoided_per_window"].append(avoided)
            metrics["wake_issued_per_window"].append(issued)
            # Percentiles are undefined for windows without scheduled pumps.
            due = re.search(r"scheduled=(\d+)", line)
            if due and int(due.group(1)) > 0:
                metrics["deadline_late_p50_ms"].append(p50)
                metrics["deadline_late_p95_ms"].append(p95)
                metrics["deadline_late_p99_ms"].append(p99)
                metrics["request_to_pump_p95_ms"].append(r95)
        if m := RE_LATENCY.search(line):
            inputs, _i50, i95, _i99, gpu, _g50, g95, _g99 = map(float, m.groups())
            if inputs > 0: metrics["input_to_ui_p95_ms"].append(i95)
            if gpu > 0: metrics["gpu_submit_to_ui_p95_ms"].append(g95)
        if m := RE_CPU.search(line):
            cpu_pct, _cpu_ms, _wall = map(float, m.groups())
            metrics["process_cpu_pct"].append(cpu_pct)
        if m := RE_SLINT.search(line):
            metrics["slint_ticks_per_s"].append(float(m.group(1)))
        if m := RE_SCHEDULER.search(line):
            metrics["wake_requested_per_window"].append(float(m.group(1)))
            metrics["wake_posted_per_window"].append(float(m.group(2)))
            metrics["wake_dispatched_per_window"].append(float(m.group(3)))
            metrics["wake_coalesced_per_window"].append(float(m.group(4)))
        if m := RE_DISPLAY.search(line):
            out["wayland"] = f"{m.group(1)} / ozone={m.group(2)} / socket={m.group(3)}"
        if "cef_shutdown_complete browsers_pending=0" in line:
            out["closing"] = True
        if "first_valid_frame" in line:
            out["first_gpu_frame"] = True
        if "switching all browsers to CPU OSR" in line:
            out["gpu_fallback"] = True
    out["metrics"] = {k: (statistics.mean(v) if v else None) for k, v in metrics.items()}
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="+", metavar="LABEL=LOGFILE", help="au moins deux journaux")
    args = parser.parse_args()
    if len(args.runs) < 2:
        parser.error("au moins deux séries sont nécessaires")
    runs = {}
    for item in args.runs:
        if "=" not in item:
            parser.error(f"attendu LABEL=LOGFILE, obtenu {item!r}")
        label, path = item.split("=", 1)
        if not label or label in runs:
            parser.error(f"libellé vide ou répété : {label!r}")
        try:
            runs[label] = read_metrics(Path(path))
        except ValueError as exc:
            parser.error(str(exc))
    width = max(26, *(len(x) + 3 for x in runs))
    print("Aster P3/P4 — comparaison exploratoire des fenêtres périodiques")
    print("Moyennes des fenêtres disponibles, sans pondération ni causalité : charges identiques nécessaires.")
    print("Percentiles : moyennes des percentiles par fenêtre, PAS des percentiles globaux.")
    print(f"{'métrique':<33}" + "".join(f"{x:>{width}}" for x in runs))
    print("-" * (33 + len(runs) * width))
    for metric in FIELDS:
        values = ["n/a" if run["metrics"][metric] is None else f"{run['metrics'][metric]:.2f}" for run in runs.values()]
        print(f"{metric:<33}" + "".join(f"{x:>{width}}" for x in values))
    print("\nStatuts (pas des mesures de vitesse) :")
    for name, run in runs.items():
        print(f"{name}: fenetres={run['windows']} display={run['wayland']} premier_GPU={run['first_gpu_frame']} repli_CPU={run['gpu_fallback']} fermeture_CEF={run['closing']}")
    print("\nCPU = processus Aster uniquement (sous-processus Chromium exclus).")
    print("input_to_ui = proxy dernière entrée -> prochaine MAJ de l'image, pas entrée -> scan-out.")
    print("gpu_submit_to_ui = commande GPU soumise -> propriété Image Slint, pas GPU -> écran.")
    print("Ne comparer une animation qu'à activité identique ; aucun gain causal n'est déduit ici.")


if __name__ == "__main__":
    main()
