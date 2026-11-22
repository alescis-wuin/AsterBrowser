"""Isolated, sequential P5 benchmark execution and anomaly collection."""
from __future__ import annotations
from contextlib import contextmanager
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
import hashlib
import json
import os
import platform
import random
import signal
import socket
import tempfile
import shutil
import subprocess
import sys
import time
import traceback

from .display import Display, DisplayError, display_session, terminate_group
from .drivers import CdpDriver, DriverError, InternalDriver
from .fixture import fixture_server
from .monitor import ProcessTreeMonitor
from .plan import Case, best_driver
from .telemetry import detect_anomalies, read_log


def free_local_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _sha256(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(block)
    return sha.hexdigest()


def _command(*args: str, cwd: Path | None = None) -> str | None:
    try:
        completed = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=3)
        if completed.returncode == 0:
            return completed.stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        pass
    return None


def environment_metadata(repo: Path, binary: Path, config: dict) -> dict:
    return {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "system": platform.platform(), "kernel": platform.release(),
        "python": platform.python_version(),
        "gpu_nvidia": _command("nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"),
        "rust": _command("rustc", "--version"),
        "git_revision": _command("git", "rev-parse", "HEAD", cwd=repo),
        "git_dirty": bool(_command("git", "status", "--porcelain", cwd=repo)),
        "binary_path": str(binary), "binary_sha256": _sha256(binary),
        "options": config,
    }


def _append_event(path: Path, event: str, **fields) -> None:
    with path.open("a", encoding="utf-8") as output:
        output.write(json.dumps({"time": time.time(), "event": event, **fields},
                                ensure_ascii=False) + "\n")


def _wait_document(internal: InternalDriver, cdp: CdpDriver | None,
                   expected_url: str, timeout: float = 10) -> bool:
    if cdp is not None:
        return cdp.ready(timeout)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        snapshot = internal.send("snapshot").get("snapshot") or {}
        active = snapshot.get("active")
        for tab in snapshot.get("tabs", []):
            if tab.get("id") == active and tab.get("url") == expected_url and not tab.get("loading"):
                return True
        time.sleep(.1)
    return False


def _base_url(fixture_url: str, scenario: str, seed: int) -> str:
    mode = "animation" if scenario == "animation" else "idle"
    return f"{fixture_url}?mode={mode}&seed={seed}"


def _navigate(url: str, driver: str, internal: InternalDriver, cdp: CdpDriver | None) -> None:
    if driver == "cdp":
        if cdp is None:
            raise DriverError("CDP required but unavailable")
        cdp.navigate(url)
    else:
        internal.send("navigate", url=url)


def _exercise(scenario: str, step: int, driver: str,
              internal: InternalDriver, cdp: CdpDriver | None,
              fixture_url: str, seed: int) -> None:
    if scenario == "scroll":
        direction = 1 if (step // 16) % 2 == 0 else -1
        if driver == "cdp":
            assert cdp is not None
            cdp.scroll(400, 280, 210 * direction)
        else:
            internal.send("scroll", x=400, y=280, dx=0, dy=210 * direction)
    elif scenario == "input":
        text = "a" if step % 2 == 0 else "b"
        if driver == "cdp":
            assert cdp is not None
            cdp.text(text)
        else:
            internal.send("text", text=text)
        if step % 6 == 0:
            if driver == "cdp":
                cdp.pointer(400, 300)
            else:
                internal.send("pointer", x=400, y=300, kind="move")
    elif scenario == "navigation":
        mode = "animation" if step % 2 else "idle"
        url = f"{fixture_url}?mode={mode}&seed={seed + step + 1}"
        _navigate(url, driver, internal, cdp)
        if not _wait_document(internal, cdp, url, timeout=8):
            raise DriverError("navigation target did not become ready")
    elif scenario == "tabs":
        snapshot = internal.send("snapshot")["snapshot"]
        tabs = snapshot["tabs"]
        if len(tabs) >= 4:
            # Close a newly created tab; preserve the initial Aster tab.
            internal.send("close_tab", tab=snapshot["active"])
        else:
            url = f"{fixture_url}?mode=idle&seed={seed + step + 1}"
            internal.send("new_tab", url=url)
        tabs = internal.send("snapshot")["snapshot"]["tabs"]
        if len(tabs) > 1 and step % 3 == 0:
            internal.send("activate_tab", tab=tabs[step % len(tabs)]["id"])
    elif scenario == "resize":
        width, height = ((960, 600) if step % 2 == 0 else (1280, 720))
        internal.send("resize", width=width, height=height)
    else:
        raise ValueError(f"unexpected interactive scenario: {scenario}")


def _run_actions(scenario: str, duration: float, frequency: float, driver: str,
                 internal: InternalDriver, cdp: CdpDriver | None,
                 fixture_url: str, seed: int, event_log: Path) -> int:
    deadline = time.monotonic() + duration
    actions = 0
    period = 1. / frequency
    if scenario in ("navigation", "tabs", "resize"):
        period = max(period, .8)
    next_action = time.monotonic()
    while True:
        now = time.monotonic()
        if now >= deadline:
            return actions
        if scenario in ("idle", "animation"):
            time.sleep(min(.1, deadline - now))
            continue
        if now < next_action:
            time.sleep(min(.05, next_action - now, deadline - now))
            continue
        began = time.monotonic()
        _exercise(scenario, actions, driver, internal, cdp, fixture_url, seed)
        _append_event(event_log, "action", scenario=scenario, driver=driver,
                      iteration=actions, elapsed_ms=round((time.monotonic()-began)*1000, 3))
        actions += 1
        next_action = max(next_action + period, time.monotonic())


def run_case(case: Case, display: Display, repo: Path, binary: Path,
             fixture_url: str, output: Path, *, warmup: float, duration: float,
             frequency: float, driver_preference: str, seed: int,
             startup_timeout: float, allow_cpu_fallback: bool) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    log_path = output / "aster.log"
    events_path = output / "actions.jsonl"
    events_path.write_text("", encoding="utf-8")
    monitor = None
    proc = None
    internal = None
    cdp = None
    start_t = time.monotonic()
    measure_begin = None
    measure_end = None
    ready = False
    actions = 0
    dom = None
    reasons: list[str] = []
    actual_driver = None
    completed = False
    returncode = None
    token = __import__("secrets").token_hex(24)
    internal_port, cdp_port = free_local_port(), free_local_port()
    while cdp_port == internal_port:
        cdp_port = free_local_port()
    cache_root = Path(tempfile.mkdtemp(prefix="aster-cef-", dir=output))
    env = display.env.copy()
    env.update({
        "ASTER_PERF": "1", "ASTER_LIFECYCLE": "1", "ASTER_FONTCONFIG_MODE": "auto",
        "ASTER_BENCH_PORT": str(internal_port), "ASTER_BENCH_TOKEN": token,
        "ASTER_CEF_CACHE_ROOT": str(cache_root.resolve()),
        "ASTER_CEF_OZONE": "x11" if "xwayland" in case.display or case.display == "host-x11" else "wayland",
    })
    url = _base_url(fixture_url, case.scenario, seed)
    _append_event(events_path, "start", scenario=case.scenario, variant=case.variant.name,
                  display=case.display, driver=driver_preference)
    try:
        actual_driver = best_driver(case.scenario, driver_preference)
        # CDP is disabled for native-only scenarios so its debugging server
        # cannot perturb CPU, memory or event-loop measurements.
        if actual_driver == "cdp":
            env["ASTER_CDP_PORT"] = str(cdp_port)
        with log_path.open("wb") as log:
            proc = subprocess.Popen([str(binary), *case.variant.args(case.display)], cwd=repo,
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                env=env, start_new_session=True)
        monitor = ProcessTreeMonitor(proc.pid)
        monitor.start()
        internal = InternalDriver(internal_port, token)
        internal.connect(startup_timeout)
        # CDP is required for the auto/cdp mode. Native-only benchmarks
        # deliberately omit CDP and its connection overhead.
        if actual_driver == "cdp":
            cdp = CdpDriver(cdp_port)
            cdp.connect(timeout=startup_timeout)
        _navigate(url, actual_driver, internal, cdp)
        ready = _wait_document(internal, cdp, url, timeout=startup_timeout)
        if not ready:
            raise DriverError("fixture did not report ready")
        # Page focus is a setup action, not part of the measured interval.
        if case.scenario == "input":
            if cdp is not None:
                cdp.evaluate("document.querySelector('#typing').focus()")
            else:
                internal.send("pointer", x=90, y=48, kind="down", button="left")
                internal.send("pointer", x=90, y=48, kind="up", button="left")
        if case.scenario == "resize":
            internal.send("resize", width=1024, height=768)
        if warmup > 0:
            _append_event(events_path, "warmup", seconds=warmup)
            time.sleep(warmup)
        if cdp is not None:
            cdp.reset_metrics()
        internal.send("mark", label="measure_begin")
        measure_begin = time.monotonic() - monitor.started
        _append_event(events_path, "measure_begin")
        actions = _run_actions(case.scenario, duration, frequency, actual_driver,
                               internal, cdp, fixture_url, seed, events_path)
        internal.send("mark", label="measure_end")
        measure_end = time.monotonic() - monitor.started
        _append_event(events_path, "measure_end", actions=actions)
        if cdp is not None:
            try:
                dom = cdp.metrics()
            except DriverError as exc:
                reasons.append(f"cdp_metrics_error:{exc}")
        completed = True
    except (DriverError, OSError, DisplayError, ValueError, subprocess.SubprocessError) as exc:
        reasons.append(f"execution_error:{exc}")
        _append_event(events_path, "error", message=str(exc))
    except Exception as exc:
        reasons.append(f"unexpected_error:{type(exc).__name__}:{exc}")
        (output / "exception.txt").write_text(traceback.format_exc(), encoding="utf-8")
    finally:
        if internal is not None and internal.connection is not None:
            try:
                internal.send("quit")
            except (DriverError, OSError):
                reasons.append("graceful_quit_failed")
            internal.close()
        if cdp is not None:
            cdp.close()
        if proc is not None:
            try:
                returncode = proc.wait(timeout=7)
            except subprocess.TimeoutExpired:
                reasons.append("browser_shutdown_timeout")
                terminate_group(proc)
                returncode = proc.poll()
        if monitor is not None:
            monitor.stop()
            monitor.write_csv(output / "process-tree.csv")
        # Never retain Chromium profiles; the benchmark artifact only contains
        # measurements and logs, not cookies or browsing state.
        shutil.rmtree(cache_root, ignore_errors=True)
    content = read_log(log_path) if log_path.exists() else {"text": "", "marked": False, "telemetry": {}}
    if not content["marked"]:
        reasons.append("measurement_markers_missing")
    if monitor is not None and measure_begin is not None and measure_end is not None:
        metrics = monitor.metrics(measure_begin, measure_end)
    else:
        metrics = {"monitor_samples": 0}
    anomalies = detect_anomalies(content["text"], renderer=case.variant.renderer,
                                 returncode=returncode, document_ready=ready, complete=True)
    if allow_cpu_fallback and case.variant.renderer == "gpu":
        # Diagnostic mode preserves all anomalies and does not promote CPU
        # fallback to a valid GPU score. It merely labels the intended use.
        if any(item in anomalies for item in (
            "no_accelerated_cef_frame", "no_gpu_renderer_setup", "gpu_fallback_or_capture_failure"
        )):
            reasons.append("diagnostic_cpu_fallback_not_comparable")
    reasons.extend(anomalies)
    result = {
        "case": case.name, "variant": case.variant.as_dict(),
        "scenario": case.scenario, "display": case.display, "repetition": case.repetition,
        "driver": actual_driver, "status": "pass" if completed and not reasons else "fail",
        "reasons": sorted(set(reasons)), "document_ready": ready,
        "returncode": returncode, "actions": actions,
        "wall_seconds": round(time.monotonic() - start_t, 3),
        "measurement_seconds": round((measure_end - measure_begin), 3)
            if measure_begin is not None and measure_end is not None else None,
        "process": metrics, "telemetry": content["telemetry"], "dom": dom,
        "files": {"log": f"{case.name}/aster.log",
                  "events": f"{case.name}/actions.jsonl",
                  "process_samples": f"{case.name}/process-tree.csv"},
    }
    (output / "case.json").write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    return result
