"""Isolated native displays, including Weston headless GL + XWayland."""
from __future__ import annotations
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
import os
import re
import shutil
import signal
import socket
import stat
import subprocess
import time
import uuid


class DisplayError(RuntimeError):
    pass


@dataclass
class Display:
    mode: str
    env: dict[str, str]
    log: Path | None = None
    weston_pid: int | None = None


def _gpu_compositor_log(log_path: Path) -> bool:
    text = log_path.read_text(errors="replace") if log_path.exists() else ""
    lowered = text.lower()
    if "llvmpipe" in lowered or "softpipe" in lowered or "software renderer" in lowered:
        return False
    if "gl renderer" in lowered or "gl-renderer" in lowered or "egl version" in lowered:
        return True
    # Not every Weston version prints a renderer line without verbose logging.
    # Do not claim confirmed hardware acceleration from an absent log.
    return False


def _xwayland_display(text: str) -> str | None:
    # Weston versions differ in capitalization and message wording.
    patterns = (
        r"[Xx](?:[Ww]ayland|server)[^\n]*?(?:display|DISPLAY)\s+(:\d+)",
        r"[Ll]istening on [Xx]11 display\s+(:\d+)",
        r"[Xx]server[^\n]*?:\s*(:\d+)",
    )
    for pattern in patterns:
        matches = re.findall(pattern, text)
        if matches:
            return matches[-1]
    return None


def _socket_ready(path: Path) -> bool:
    try:
        return stat.S_ISSOCK(path.stat().st_mode)
    except (FileNotFoundError, OSError):
        return False


def terminate_group(proc: subprocess.Popen, *, grace_s: float = 3) -> None:
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=grace_s)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=2)


@contextmanager
def display_session(mode: str, log_dir: Path, width: int, height: int):
    env = os.environ.copy()
    if mode == "host-x11":
        if not env.get("DISPLAY"):
            raise DisplayError("DISPLAY absent: host-x11 cannot start")
        env.pop("WAYLAND_DISPLAY", None)
        env["XDG_SESSION_TYPE"] = "x11"
        yield Display(mode, env)
        return
    if mode == "host-wayland":
        if not env.get("WAYLAND_DISPLAY") or not env.get("XDG_RUNTIME_DIR"):
            raise DisplayError("Wayland socket absent: host-wayland cannot start")
        socket_path = Path(env["XDG_RUNTIME_DIR"]) / env["WAYLAND_DISPLAY"]
        if not _socket_ready(socket_path):
            raise DisplayError(f"Wayland socket unavailable: {socket_path}")
        env.pop("DISPLAY", None)
        env["XDG_SESSION_TYPE"] = "wayland"
        yield Display(mode, env)
        return
    if mode not in ("weston-wayland", "weston-xwayland"):
        raise DisplayError(f"unknown display mode: {mode}")
    if not shutil.which("weston"):
        raise DisplayError("Weston not installed; install weston and xorg-xwayland if needed")
    runtime_dir = env.get("XDG_RUNTIME_DIR")
    if not runtime_dir or not Path(runtime_dir).is_dir():
        raise DisplayError("XDG_RUNTIME_DIR missing (requires a user graphical session)")
    socket_name = f"aster-bench-{os.getpid()}-{uuid.uuid4().hex[:8]}"
    socket_path = Path(runtime_dir) / socket_name
    weston_log = log_dir / "weston.log"
    console_log = log_dir / "weston-console.log"
    weston_env = env.copy()
    weston_env.pop("WAYLAND_DISPLAY", None)
    weston_env.pop("DISPLAY", None)
    command = [
        "weston", "--backend=headless", "--renderer=gl",
        f"--socket={socket_name}", f"--width={width}", f"--height={height}",
        f"--log={weston_log}",
    ]
    if mode == "weston-xwayland":
        if not shutil.which("Xwayland"):
            raise DisplayError("Xwayland not found; install xorg-xwayland")
        command.append("--xwayland")
    with console_log.open("wb") as output:
        proc = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output,
                                stderr=subprocess.STDOUT, env=weston_env, start_new_session=True)
    try:
        deadline = time.monotonic() + 12
        xdisplay: str | None = None
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                raise DisplayError(f"Weston exited with status {proc.returncode}; see {weston_log}")
            if _socket_ready(socket_path):
                if mode == "weston-wayland":
                    break
                text = weston_log.read_text(errors="replace") if weston_log.exists() else ""
                text += console_log.read_text(errors="replace") if console_log.exists() else ""
                xdisplay = _xwayland_display(text)
                if xdisplay and _socket_ready(Path("/tmp/.X11-unix") / f"X{xdisplay[1:]}"):
                    break
            time.sleep(.1)
        else:
            raise DisplayError(f"Weston socket/XWayland display not ready; see {weston_log}")
        if not _gpu_compositor_log(weston_log):
            raise DisplayError("Weston GL renderer not verified (possibly software); see weston.log")
        env["XDG_RUNTIME_DIR"] = runtime_dir
        if mode == "weston-wayland":
            env["WAYLAND_DISPLAY"] = socket_name
            env.pop("DISPLAY", None)
            env["XDG_SESSION_TYPE"] = "wayland"
        else:
            env.pop("WAYLAND_DISPLAY", None)
            env["DISPLAY"] = xdisplay  # parsed from THIS Weston instance only
            env["XDG_SESSION_TYPE"] = "x11"
        yield Display(mode, env, weston_log, proc.pid)
    finally:
        terminate_group(proc)
        # Weston owns the socket; it normally removes it during shutdown.
        # Never unlink an arbitrary socket belonging to another process.
