"""Standalone JSON/CSV/HTML reports, with paired-by-scenario regression flags."""
from __future__ import annotations
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
import csv
import html
import json
from pathlib import Path
import statistics

KEY_METRICS = (
    "cpu_total_core_pct", "cpu_aster_core_pct", "cpu_chromium_core_pct", "rss_peak_mib",
    "gpu_imported", "gpu_importer_full_copy", "gpu_presentation_writes",
    "gpu_write_mib_per_imported_frame", "gpu_fullcopy_mib_per_imported_frame",
    "slint_ticks", "pump_calls", "pump_deadline_late_p95_ms",
    "latency_input_to_ui_p95_ms", "latency_gpu_submit_to_ui_p95_ms",
)
LOWER_IS_BETTER = {"cpu_total_core_pct", "cpu_aster_core_pct", "cpu_chromium_core_pct",
    "rss_peak_mib", "gpu_importer_full_copy", "gpu_presentation_writes",
    "gpu_write_mib_per_imported_frame", "gpu_fullcopy_mib_per_imported_frame",
    "pump_calls", "pump_deadline_late_p95_ms", "latency_input_to_ui_p95_ms",
    "latency_gpu_submit_to_ui_p95_ms"}


def flatten(case: dict) -> dict:
    values = {"case": case["case"], "status": case["status"],
              "variant": case["variant"]["name"], "scenario": case["scenario"],
              "display": case["display"], "repetition": case["repetition"],
              "driver": case.get("driver"), "actions": case.get("actions"),
              "measurement_seconds": case.get("measurement_seconds"),
              "anomalies": ";".join(case.get("reasons", []))}
    values.update({key: val for key, val in case.get("process", {}).items()})
    values.update(case.get("telemetry", {}).get("averages", {}))
    frame_rate = values.get("gpu_imported")
    if isinstance(frame_rate, (int, float)) and frame_rate > 0:
        for source, dest in (
            ("gpu_presentation_writes", "gpu_write_mib_per_imported_frame"),
            ("gpu_importer_full_copy", "gpu_fullcopy_mib_per_imported_frame"),
        ):
            if isinstance(values.get(source), (int, float)):
                values[dest] = round(values[source] / frame_rate, 5)
    if case.get("dom"):
        values.update({f"dom_{key}": val for key, val in case["dom"].items()
                       if isinstance(val, (int, float))})
    return values


def aggregate(cases: list[dict]) -> list[dict]:
    grouped = defaultdict(list)
    for item in cases:
        grouped[(item["display"], item["scenario"], item["variant"]["name"])].append(item)
    summaries = []
    for (display, scenario, variant), items in sorted(grouped.items()):
        passes = [flatten(item) for item in items if item["status"] == "pass"]
        medians = {}
        for field in KEY_METRICS:
            present = [float(row[field]) for row in passes if row.get(field) is not None]
            if present:
                medians[field] = round(statistics.median(present), 4)
        summaries.append({"display": display, "scenario": scenario, "variant": variant,
            "passes": len(passes), "fails": len(items) - len(passes),
            "repetitions": len(items), "median": medians})
    return summaries


def detect_regressions(summaries: list[dict], threshold: float = .15,
                       minimum_repeats: int = 2) -> list[dict]:
    if threshold <= 0:
        raise ValueError("regression threshold must be positive")
    lookup = {(s["display"], s["scenario"], s["variant"]): s for s in summaries}
    regressions = []
    for summary in summaries:
        name = summary["variant"]
        if name.startswith("gpu_"):
            baseline = "gpu_legacy_2"
        elif name.startswith("loop_"):
            baseline = "loop_legacy"
        elif name.startswith("cart_"):
            baseline = "cart_legacy2_legacy33_poll"
        else:
            continue
        if name == baseline or summary["passes"] < minimum_repeats:
            continue
        previous = lookup.get((summary["display"], summary["scenario"], baseline))
        if previous is None or previous["passes"] < minimum_repeats:
            continue
        for metric in LOWER_IS_BETTER:
            old = previous["median"].get(metric)
            new = summary["median"].get(metric)
            if old is None or new is None or old <= 0:
                continue
            delta = (new - old) / old
            if delta > threshold:
                regressions.append({"display": summary["display"],
                    "scenario": summary["scenario"], "variant": name,
                    "baseline": baseline, "metric": metric,
                    "delta_pct": round(delta * 100, 2),
                    "baseline_value": old, "measured_value": new})
    return regressions


def write_reports(output: Path, cases: list[dict], metadata: dict,
                  *, regression_threshold: float = .15) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    summaries = aggregate(cases)
    regressions = detect_regressions(summaries, regression_threshold)
    result = {
        "schema": "aster-p5/v1", "generated_utc": datetime.now(timezone.utc).isoformat(),
        "metadata": metadata, "cases": cases, "groups": summaries,
        "regressions": regressions,
        "summary": {"cases": len(cases),
            "passed": sum(c["status"] == "pass" for c in cases),
            "failed": sum(c["status"] != "pass" for c in cases),
            "regressions": len(regressions)},
    }
    (output / "report.json").write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    flat = [flatten(case) for case in cases]
    columns = sorted({key for item in flat for key in item})
    with (output / "results.csv").open("w", newline="", encoding="utf-8") as file:
        writer = csv.DictWriter(file, fieldnames=columns)
        writer.writeheader()
        writer.writerows(flat)
    (output / "index.html").write_text(_html_report(result), encoding="utf-8")
    return result


def _html_report(result: dict) -> str:
    e = html.escape
    summary = result["summary"]
    rows = []
    for item in result["groups"]:
        median = item["median"]
        def value(name):
            n = median.get(name)
            return "—" if n is None else f"{n:.2f}"
        rows.append("<tr>" + "".join(f"<td>{e(str(v))}</td>" for v in (
            item["display"], item["scenario"], item["variant"],
            f'{item["passes"]}/{item["repetitions"]}', value("cpu_total_core_pct"),
            value("rss_peak_mib"), value("gpu_presentation_writes"),
            value("pump_calls"), value("latency_input_to_ui_p95_ms"))) + "</tr>")
    details = []
    for case in result["cases"]:
        causes = "; ".join(case.get("reasons", [])) or "—"
        log = e(case["files"]["log"])
        events = e(case["files"]["events"])
        sample = e(case["files"]["process_samples"])
        details.append(f'<tr><td>{e(case["case"])}</td><td>{e(case["status"])}</td>'
            f'<td>{e(causes)}</td><td><a href="{log}">log</a> · '
            f'<a href="{events}">actions</a> · <a href="{sample}">processus</a></td></tr>')
    regressions = "".join(f'<tr><td>{e(r["display"])}</td><td>{e(r["scenario"])}</td>'
        f'<td>{e(r["variant"])}</td><td>{e(r["metric"])}</td>'
        f'<td>+{r["delta_pct"]:.1f}%</td></tr>' for r in result["regressions"])
    # Small inline SVG bar chart; no external scripts, charts remain readable offline.
    gpu_data = [(f'{g["variant"]} · {g["scenario"]}', g["median"]["gpu_presentation_writes"])
                for g in result["groups"] if "gpu_presentation_writes" in g["median"]][:32]
    max_value = max((v for _, v in gpu_data), default=1) or 1
    bars = []
    for i, (name, value) in enumerate(gpu_data):
        y = 20 + i * 26
        width = 450 * value / max_value
        bars.append(f'<text x="0" y="{y+12}">{e(name[:48])}</text>'
                    f'<rect x="365" y="{y}" width="{width:.1f}" height="17" rx="3" fill="#3b82ac"/>'
                    f'<text x="{375+width:.1f}" y="{y+12}">{value:.2f}</text>')
    svg = f'<svg role="img" aria-label="Écritures GPU en Mio par seconde" viewBox="0 0 980 {max(60, len(gpu_data)*26+40)}">' + "".join(bars) + "</svg>"
    return f'''<!doctype html><html lang="fr"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Aster P5 — benchmarks</title>
<style>body{{font:14px/1.55 system-ui,sans-serif;max-width:1500px;margin:40px auto;padding:0 24px;color:#122231}}
h1,h2{{margin-top:1.5em}}table{{width:100%;border-collapse:collapse;font-size:12px}}
th,td{{padding:8px;border-bottom:1px solid #dde5ee;text-align:left;vertical-align:top}}th{{background:#eef3f7;position:sticky;top:0}}
svg{{width:100%;height:auto;min-height:80px}}svg text{{font:11px system-ui;fill:#122231}}
a{{color:#125b8c}}.muted{{color:#555}}.scroll{{overflow:auto;max-height:600px}}</style>
<h1>Aster Browser — P5</h1>
<p><strong>{summary["passed"]}</strong> réussis · <strong>{summary["failed"]}</strong> échecs ·
<strong>{summary["regressions"]}</strong> signaux de régression (comparaison exploratoire).</p>
<p class="muted">Les moyennes de télémétrie sont calculées sur les fenêtres périodiques entre marqueurs ;
les P95/P99 internes sont des moyennes de percentiles de fenêtre, pas des percentiles globaux.
Le CPU agrège Aster et ses processus enfants. RSS cumulé non dédupliqué.</p>
<p><a href="report.json">Données JSON</a> · <a href="results.csv">Résultats CSV</a></p>
<h2>Mesures médianes par configuration</h2>
<div class="scroll"><table><thead><tr><th>Affichage</th><th>Scénario</th><th>Configuration</th><th>Succès</th>
<th>CPU total (% cœur)</th><th>RSS max (Mio)</th><th>Écritures GPU (Mio/s)</th><th>Appels CEF/s</th><th>Latence entrée→UI P95 (ms)</th></tr></thead>
<tbody>{''.join(rows)}</tbody></table></div>
<h2>Écritures de présentation GPU</h2>{svg}
<h2>Régressions supérieures au seuil</h2>
<table><thead><tr><th>Affichage</th><th>Scénario</th><th>Variante</th><th>Indicateur</th><th>Écart</th></tr></thead>
<tbody>{regressions or '<tr><td colspan="5">Aucune détectée selon les répétitions disponibles.</td></tr>'}</tbody></table>
<h2>Essais et journaux</h2><div class="scroll"><table><thead><tr><th>Essai</th><th>État</th>
<th>Anomalies</th><th>Fichiers</th></tr></thead><tbody>{''.join(details)}</tbody></table></div>
</html>'''
