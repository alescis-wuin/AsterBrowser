"""Wall-clock CPU and memory sampling for Aster AND all its Chromium descendants."""
from __future__ import annotations
import csv
from dataclasses import asdict, dataclass
from pathlib import Path
import statistics
import threading
import time
import psutil


@dataclass
class Sample:
    elapsed_s: float
    cpu_core_pct: float
    aster_cpu_core_pct: float
    children_cpu_core_pct: float
    rss_mib_sum: float
    process_count: int
    chromium_children: int
    gpu_processes: int


class ProcessTreeMonitor:
    def __init__(self, pid: int, interval: float = .25):
        self.pid = pid
        self.interval = interval
        self.samples: list[Sample] = []
        self.started = time.monotonic()
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._previous: dict[tuple[int, float], float] = {}
        self._last_time: float | None = None

    def start(self) -> None:
        if self._thread is not None:
            raise RuntimeError("monitor already started")
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        if self._thread is not None:
            self._thread.join(timeout=3)

    def _loop(self) -> None:
        while not self._stop_event.is_set():
            self.sample()
            self._stop_event.wait(self.interval)

    def sample(self) -> None:
        try:
            root = psutil.Process(self.pid)
            processes = [root, *root.children(recursive=True)]
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            return
        now = time.monotonic()
        readings: dict[tuple[int, float], float] = {}
        rss = 0
        children_count = 0
        gpu_count = 0
        root_key = None
        for process in processes:
            try:
                # create_time disambiguates PID reuse.
                key = (process.pid, process.create_time())
                usage = process.cpu_times()
                readings[key] = usage.user + usage.system
                rss += process.memory_info().rss
                if process.pid == self.pid:
                    root_key = key
                else:
                    children_count += 1
                    if "--type=gpu-process" in process.cmdline():
                        gpu_count += 1
            except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.ZombieProcess):
                continue
        if self._last_time is not None:
            elapsed = max(now - self._last_time, 1e-9)
            deltas = {key: max(0., cpu - self._previous.get(key, cpu))
                      for key, cpu in readings.items()}
            total = sum(deltas.values()) * 100 / elapsed
            main = deltas.get(root_key, 0.) * 100 / elapsed
            self.samples.append(Sample(now - self.started, total, main, max(0., total-main),
                rss / 1048576, len(readings), children_count, gpu_count))
        self._last_time = now
        self._previous = readings

    def metrics(self, begin_s: float, end_s: float) -> dict:
        series = [s for s in self.samples if begin_s <= s.elapsed_s <= end_s]
        if not series:
            return {"monitor_samples": 0}
        def p95(items):
            ordered = sorted(items)
            return ordered[int((len(ordered) - 1) * .95 + .999999)]
        def summary(field):
            values = [float(getattr(s, field)) for s in series]
            return statistics.mean(values), p95(values), max(values)
        cpu, cpu95, cpu_max = summary("cpu_core_pct")
        main, _, _ = summary("aster_cpu_core_pct")
        children, _, _ = summary("children_cpu_core_pct")
        rss, rss95, rss_max = summary("rss_mib_sum")
        return {"monitor_samples": len(series), "cpu_total_core_pct": round(cpu, 3),
            "cpu_total_p95_pct": round(cpu95, 3), "cpu_total_peak_pct": round(cpu_max, 3),
            "cpu_aster_core_pct": round(main, 3), "cpu_chromium_core_pct": round(children, 3),
            "rss_total_mib": round(rss, 3), "rss_p95_mib": round(rss95, 3),
            "rss_peak_mib": round(rss_max, 3),
            "max_process_count": max(s.process_count for s in series),
            "max_chromium_children": max(s.chromium_children for s in series),
            "max_gpu_processes": max(s.gpu_processes for s in series)}

    def write_csv(self, path: Path) -> None:
        if not self.samples:
            path.write_text("elapsed_s,cpu_core_pct,aster_cpu_core_pct,children_cpu_core_pct,rss_mib_sum,process_count,chromium_children,gpu_processes\n")
            return
        with path.open("w", newline="", encoding="utf-8") as output:
            writer = csv.DictWriter(output, fieldnames=list(asdict(self.samples[0])))
            writer.writeheader()
            for sample in self.samples:
                writer.writerow(asdict(sample))
