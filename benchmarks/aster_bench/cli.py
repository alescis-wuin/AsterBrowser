"""Command-line entrypoint for reproducible Aster benchmarks."""
from __future__ import annotations
import argparse
from contextlib import ExitStack
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys

from .display import DisplayError, display_session
from .fixture import fixture_server
from .plan import DISPLAYS, SCENARIOS, matrix, variants
from .report import write_reports
from .runner import environment_metadata, run_case

ROOT = Path(__file__).resolve().parents[2]


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="aster-bench",
        description="Benchmarks locaux Aster CEF/WGPU/Slint avec écran virtuel et contrôles CDP/natifs")
    p.add_argument("command", choices=("plan", "run"), help="plan: aucune exécution; run: lancer le banc")
    p.add_argument("--repo", type=Path, default=ROOT)
    p.add_argument("--binary", type=Path, default=Path("target/release/aster-desktop"))
    p.add_argument("--suite", choices=("gpu", "loop", "cpu", "all", "cartesian"), default="all")
    p.add_argument("--displays", default="host-x11",
                   help=f"Modes séparés par virgule : {','.join(DISPLAYS)}")
    p.add_argument("--scenarios", default=",".join(SCENARIOS))
    p.add_argument("--repetitions", type=int, default=5)
    p.add_argument("--seed", type=int, default=20261122)
    p.add_argument("--no-shuffle", action="store_true")
    p.add_argument("--max-cases", type=int, default=500)
    p.add_argument("--warmup", type=float, default=10.)
    p.add_argument("--duration", type=float, default=30.)
    p.add_argument("--action-rate", type=float, default=10.)
    p.add_argument("--launch-timeout", type=float, default=20.)
    p.add_argument("--width", type=int, default=1280)
    p.add_argument("--height", type=int, default=800)
    p.add_argument("--driver", choices=("auto", "cdp", "internal"), default="auto")
    p.add_argument("--output", type=Path,
                   default=ROOT / "benchmarks" / "results" /
                      datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    p.add_argument("--skip-build", action="store_true", help="Reuse prebuilt cef-gpu,benchmark-control binary")
    p.add_argument("--allow-cpu-fallback", action="store_true",
                   help="Diagnostic only: fallback results stay non-comparable to GPU results")
    p.add_argument("--regression-threshold", type=float, default=.15)
    p.add_argument("--fail-on-regression", action="store_true")
    return p


def _csv(value: str) -> list[str]:
    return list(dict.fromkeys(part.strip() for part in value.split(",") if part.strip()))


def main(argv=None) -> int:
    args = parser().parse_args(argv)
    displays, scenarios = _csv(args.displays), _csv(args.scenarios)
    try:
        cases = matrix(args.suite, displays, scenarios, args.repetitions,
                       args.seed, shuffle=not args.no_shuffle)
        if args.max_cases < 1 or len(cases) > args.max_cases:
            raise ValueError(f"{len(cases)} cases exceeds --max-cases={args.max_cases}")
        if not (0 <= args.warmup <= 300 and 1 <= args.duration <= 3600):
            raise ValueError("warmup must be 0..300s and measurement duration 1..3600s")
        if not (.1 <= args.action_rate <= 100):
            raise ValueError("action rate must be 0.1..100Hz")
        if not (640 <= args.width <= 3840 and 480 <= args.height <= 2160):
            raise ValueError("resolution must be 640..3840 x 480..2160")
        if args.driver == "cdp" and any(c.scenario in ("tabs", "resize") for c in cases):
            raise ValueError("CDP cannot manipulate Aster native tabs or Slint resizing; use --driver auto/internal")
        if args.regression_threshold <= 0:
            raise ValueError("--regression-threshold must be positive")
    except ValueError as exc:
        print(f"aster-bench: {exc}", file=sys.stderr)
        return 2
    if args.command == "plan":
        print(json.dumps({"cases": len(cases), "suite": args.suite,
            "matrix": [{"name": c.name, "display": c.display,
                        "scenario": c.scenario, "variant": c.variant.as_dict(),
                        "repetition": c.repetition} for c in cases]}, indent=2))
        return 0

    repo = args.repo.resolve()
    binary = (repo / args.binary).resolve() if not args.binary.is_absolute() else args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if not args.skip_build:
        command = ["cargo", "build", "--release", "-p", "aster-desktop",
                   "--features", "cef-gpu,benchmark-control"]
        print("Build:", " ".join(command), flush=True)
        try:
            subprocess.run(command, cwd=repo, check=True)
        except (OSError, subprocess.CalledProcessError) as exc:
            print(f"aster-bench: build failed: {exc}", file=sys.stderr)
            return 2
    if not binary.is_file():
        print(f"aster-bench: binary not found: {binary}", file=sys.stderr)
        return 2
    config = {"suite": args.suite, "displays": displays, "scenarios": scenarios,
              "repetitions": args.repetitions, "seed": args.seed, "warmup": args.warmup,
              "duration": args.duration, "driver": args.driver, "action_rate": args.action_rate,
              "width": args.width, "height": args.height, "shuffle": not args.no_shuffle}
    metadata = environment_metadata(repo, binary, config)
    results = []
    try:
        with ExitStack() as stack:
            displays_ready = {}
            for display_name in displays:
                path = output / "displays" / display_name
                path.mkdir(parents=True, exist_ok=True)
                displays_ready[display_name] = stack.enter_context(
                    display_session(display_name, path, args.width, args.height))
            fixture = stack.enter_context(fixture_server())
            for i, case in enumerate(cases, start=1):
                print(f"[{i}/{len(cases)}] {case.name}", flush=True)
                result = run_case(case, displays_ready[case.display], repo, binary,
                    fixture, output / case.name, warmup=args.warmup, duration=args.duration,
                    frequency=args.action_rate, driver_preference=args.driver,
                    seed=args.seed, startup_timeout=args.launch_timeout,
                    allow_cpu_fallback=args.allow_cpu_fallback)
                print(f"  {result['status']}  CPU={result['process'].get('cpu_total_core_pct', 'n/a')}% "
                      f"anomalies={','.join(result['reasons']) or 'none'}", flush=True)
                results.append(result)
    except KeyboardInterrupt:
        print("Benchmark interrupted; writing partial report", file=sys.stderr)
    except DisplayError as exc:
        print(f"aster-bench: display startup failed: {exc}", file=sys.stderr)
    finally:
        report = write_reports(output, results, metadata,
                               regression_threshold=args.regression_threshold)
        print(f"Reports: {output / 'index.html'} | {output / 'report.json'} | {output / 'results.csv'}")
        print(f"Passed={report['summary']['passed']} Failed={report['summary']['failed']} "
              f"regressions={report['summary']['regressions']}")
    if len(results) < len(cases) or report["summary"]["failed"]:
        return 1
    if args.fail_on_regression and report["summary"]["regressions"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
