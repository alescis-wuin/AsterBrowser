"""CEF/CDP and opt-in Aster native-control drivers. Localhost only."""
from __future__ import annotations
import json
import socket
import time
from urllib.request import urlopen
import websocket


class DriverError(RuntimeError):
    pass


class InternalDriver:
    def __init__(self, port: int, token: str):
        self.port = port
        self.token = token
        self.seq = 0
        self.connection: socket.socket | None = None
        self.reader = None

    def connect(self, timeout: float = 10) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                self.connection = socket.create_connection(("127.0.0.1", self.port), timeout=.5)
                self.connection.settimeout(5)
                self.reader = self.connection.makefile("rb")
                self.send("ping")
                return
            except (OSError, DriverError):
                self.close()
                time.sleep(.1)
        raise DriverError("Aster native-control endpoint unavailable; compile with --features benchmark-control")

    def send(self, action: str, **kwargs) -> dict:
        if self.connection is None or self.reader is None:
            raise DriverError("native-control socket is not connected")
        self.seq += 1
        request = {"id": self.seq, "token": self.token, "action": action, **kwargs}
        try:
            data = (json.dumps(request, separators=(",", ":")) + "\n").encode("utf-8")
            self.connection.sendall(data)
            line = self.reader.readline(65536)
            response = json.loads(line)
        except (OSError, ValueError, TypeError) as exc:
            raise DriverError(f"native-control protocol failure: {exc}") from exc
        if response.get("id") != self.seq:
            raise DriverError("native-control reply id mismatch")
        if not response.get("ok"):
            raise DriverError(f"native-control rejected {action}: {response.get('error')}")
        return response

    def close(self) -> None:
        if self.reader is not None:
            self.reader.close()
            self.reader = None
        if self.connection is not None:
            self.connection.close()
            self.connection = None


class CdpDriver:
    def __init__(self, port: int):
        self.port = port
        self.ws = None
        self.seq = 0

    def connect(self, timeout: float = 10, target_url: str | None = None) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                with urlopen(f"http://127.0.0.1:{self.port}/json/list", timeout=.5) as response:
                    targets = json.load(response)
                pages = [target for target in targets if target.get("type") == "page"
                         and target.get("webSocketDebuggerUrl")]
                if not pages:
                    raise DriverError("CEF has no attachable CDP page target")
                if target_url is not None:
                    pages.sort(key=lambda target: target.get("url") != target_url)
                url = pages[0]["webSocketDebuggerUrl"]
                # Chromium may reject a synthetic Origin header. websocket-client
                # supports suppress_origin, so we never need a wildcard origins flag.
                self.ws = websocket.create_connection(url, timeout=5, suppress_origin=True)
                self.ws.settimeout(5)
                self.call("Page.enable")
                self.call("Runtime.enable")
                return
            except (DriverError, OSError, ValueError, websocket.WebSocketException):
                self.close()
                time.sleep(.15)
        raise DriverError("CEF remote debugging unavailable; verify ASTER_CDP_PORT and CEF logs")

    def call(self, method: str, params: dict | None = None) -> dict:
        if self.ws is None:
            raise DriverError("CDP websocket is not connected")
        self.seq += 1
        sequence = self.seq
        self.ws.send(json.dumps({"id": sequence, "method": method, "params": params or {}}))
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            try:
                result = json.loads(self.ws.recv())
            except (OSError, ValueError, websocket.WebSocketException) as exc:
                raise DriverError(f"CDP {method} failed: {exc}") from exc
            if result.get("id") != sequence:
                continue  # unsolicited CDP event
            if "error" in result:
                raise DriverError(f"CDP {method}: {result['error']}")
            return result.get("result", {})
        raise DriverError(f"CDP {method} response timed out")

    def evaluate(self, expression: str):
        result = self.call("Runtime.evaluate", {"expression": expression,
                     "returnByValue": True, "awaitPromise": False})
        if result.get("exceptionDetails"):
            raise DriverError(f"CDP JavaScript exception: {result['exceptionDetails']}")
        return result.get("result", {}).get("value")

    def navigate(self, url: str) -> None:
        result = self.call("Page.navigate", {"url": url})
        if result.get("errorText"):
            raise DriverError(f"Page.navigate: {result['errorText']}")

    def ready(self, timeout: float = 10) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if self.evaluate("Boolean(window.__asterBench && window.__asterBench.ready)"):
                    return True
            except DriverError:
                pass
            time.sleep(.1)
        return False

    def scroll(self, x: int, y: int, dy: int) -> None:
        self.call("Input.dispatchMouseEvent", {
            "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": dy,
        })

    def text(self, text: str) -> None:
        self.call("Input.insertText", {"text": text})

    def pointer(self, x: int, y: int) -> None:
        for kind in ("mousePressed", "mouseReleased"):
            self.call("Input.dispatchMouseEvent", {"type": kind, "x": x, "y": y,
                "button": "left", "clickCount": 1})

    def metrics(self) -> dict | None:
        value = self.evaluate("window.__asterBench ? window.__asterBench.metrics() : null")
        return value if isinstance(value, dict) else None

    def reset_metrics(self) -> None:
        self.evaluate("window.__asterBench && window.__asterBench.reset()")

    def close(self) -> None:
        if self.ws is not None:
            try:
                self.ws.close()
            except OSError:
                pass
            self.ws = None
