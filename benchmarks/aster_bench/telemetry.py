"""Parse P1-P4 log windows and reject invalid GPU or lifecycle results."""
from __future__ import annotations
from collections import defaultdict
from pathlib import Path
import re
import statistics

# These metrics are already periodically reported by Aster P1-P4.
FIELDS = {
    "pump": ("checks", "calls", "scheduled", "fallback", "wake_avoided",
             "wake_issued", "deadline_late_p50_ms", "deadline_late_p95_ms",
             "deadline_late_p99_ms", "pump_avg", "pump_max"),
    "slint": ("ticks", "frames_delivered", "pump_avg", "pump_max"),
    "gpu": ("imported", "importer_full_copy", "presentation_writes",
            "full_regions", "partial_regions", "import_wall_avg", "plan_avg",
            "submission_cpu_avg", "sync_wait_avg", "import_filter_skips"),
    "cef": ("paint", "dirty_area_pct", "copied", "emitted", "paint_cpu_avg"),
    "latency": ("input_to_ui_samples", "input_to_ui_p50_ms", "input_to_ui_p95_ms",
                "input_to_ui_p99_ms", "gpu_submit_to_ui_samples", "gpu_submit_to_ui_p95_ms"),
    "scheduler": ("wake_requested", "wake_posted", "wake_dispatched", "wake_coalesced"),
}
NUMERIC = re.compile(r"\b([a-z_][a-z_0-9]*)=(-?\d+(?:\.\d+)?)(?:MiB/s|Mpx/s|ms|/s|pct|%)?")
PREFIX = re.compile(r"\[aster:(\w+)\] (.*)")


def measurement_text(full_text: str) -> str:
    begin = "[aster:benchmark] mark=measure_begin"
    end = "[aster:benchmark] mark=measure_end"
    i = full_text.find(begin)
    if i < 0:
        return ""
    j = full_text.find(end, i + len(begin))
    if j < 0:
        return ""
    return full_text[i + len(begin):j]


def telemetry(text: str) -> dict:
    values = defaultdict(list)
    for line in text.splitlines():
        found = PREFIX.search(line)
        if not found:
            continue
        section, payload = found.groups()
        if section not in FIELDS:
            continue
        parsed = {key: float(value) for key, value in NUMERIC.findall(payload)}
        for field in FIELDS[section]:
            if field not in parsed:
                continue
            # Aster prints P50/P95/P99=0 when it collected no samples;
            # zero must not be interpreted as an actual zero-latency result.
            if (section == "latency" and field.startswith("input_to_ui_p")
                    and parsed.get("input_to_ui_samples", 0) == 0):
                continue
            if (section == "latency" and field.startswith("gpu_submit_to_ui_p")
                    and parsed.get("gpu_submit_to_ui_samples", 0) == 0):
                continue
            values[f"{section}_{field}"].append(parsed[field])
    # Percentiles here are means of two-second window estimates, not pooled
    # percentiles. Keep the raw samples and mark the limitation in reports.
    return {"averages": {key: round(statistics.mean(v), 4) for key, v in values.items()},
            "window_counts": {key: len(v) for key, v in values.items()}}


def detect_anomalies(text: str, *, renderer: str, returncode: int | None,
                     document_ready: bool, complete: bool = True) -> list[str]:
    problems = []
    if returncode not in (0, None):
        problems.append(f"browser_exit_{returncode}")
    if not document_ready:
        problems.append("document_not_ready")
    if renderer == "gpu":
        if "[aster:gpu] first_valid_frame" not in text:
            problems.append("no_accelerated_cef_frame")
        if "[aster:lifecycle] gpu_renderer_setup device_ready=1" not in text:
            problems.append("no_gpu_renderer_setup")
        if re.search(r"\[aster:gpu\].*(GPU unavailable|capture_failed_or_unavailable)", text):
            problems.append("gpu_fallback_or_capture_failure")
    if "Fontconfig error: Cannot load default config file" in text:
        problems.append("fontconfig_error")
    if "Unable to allocate frame for first frame capture" in text:
        problems.append("cef_capture_allocation_failed")
    if complete and "[aster:lifecycle] cef_shutdown_complete browsers_pending=0" not in text:
        problems.append("missing_cef_shutdown_ack")
    if "thread 'main' panicked" in text or "stack backtrace:" in text:
        problems.append("rust_panic")
    return problems


def read_log(path: Path) -> dict:
    text = path.read_text(errors="replace")
    window = measurement_text(text)
    return {"marked": bool(window), "telemetry": telemetry(window), "text": text}
