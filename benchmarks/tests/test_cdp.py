"""Network-level test of the CDP websocket transport, not mocked method calls."""
import base64
import hashlib
import json
from http.server import HTTPServer, BaseHTTPRequestHandler
import socket
import struct
import threading
import unittest
from aster_bench.drivers import CdpDriver


class CdpNetworkTests(unittest.TestCase):
    def test_discovery_connect_readiness_and_input_injection(self):
        ws = socket.socket()
        ws.bind(('127.0.0.1', 0))
        ws.listen(1)
        ws_port = ws.getsockname()[1]
        methods = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                assert self.path == '/json/list'
                data = json.dumps([{'type':'page','url':'about:blank',
                    'webSocketDebuggerUrl':f'ws://127.0.0.1:{ws_port}/devtools/page/1'}]).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, *args):
                pass

        http = HTTPServer(('127.0.0.1', 0), Handler)
        http_thread = threading.Thread(target=http.serve_forever, daemon=True)
        http_thread.start()

        def reply(sock, value):
            data = json.dumps(value).encode('utf-8')
            prefix = bytes([0x81, len(data)]) if len(data) < 126 else b'\x81\x7e' + struct.pack('!H',len(data))
            sock.sendall(prefix + data)

        def read_frame(sock):
            hdr = sock.recv(2)
            if not hdr:
                return None
            if hdr[0] & 0x0f == 0x8:  # websocket close control frame
                return None
            n = hdr[1] & 127
            if n == 126: n = struct.unpack('!H', sock.recv(2))[0]
            elif n == 127: n = struct.unpack('!Q', sock.recv(8))[0]
            key = sock.recv(4) if hdr[1] & 128 else bytes(4)
            body = bytearray()
            while len(body) < n:
                block = sock.recv(n-len(body))
                if not block: return None
                body.extend(block)
            return json.loads(bytes(v ^ key[i % 4] for i,v in enumerate(body)).decode())

        def serve_ws():
            conn, _ = ws.accept()
            with conn:
                request = b''
                while b'\r\n\r\n' not in request:
                    request += conn.recv(4096)
                headers = dict(line.split(': ',1) for line in request.decode().split('\r\n')[1:]
                               if ': ' in line)
                key = headers['Sec-WebSocket-Key']
                accept = base64.b64encode(hashlib.sha1(
                    (key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                conn.sendall(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n'
                              'Connection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\n\r\n').encode())
                while True:
                    message = read_frame(conn)
                    if message is None: break
                    name = message['method']
                    methods.append(name)
                    # Unsolicited CDP event must not be mistaken for a response.
                    reply(conn, {'method':'Runtime.consoleAPICalled','params':{}})
                    value = None
                    if name == 'Runtime.evaluate':
                        expr = message['params']['expression']
                        if 'metrics()' in expr: value = {'frames': 31}
                        elif 'Boolean(window.__asterBench' in expr: value = True
                        else: value = True
                    result = {'result': {'value': value}} if name == 'Runtime.evaluate' else {}
                    reply(conn, {'id':message['id'],'result':result})

        ws_thread = threading.Thread(target=serve_ws, daemon=True)
        ws_thread.start()
        cdp = CdpDriver(http.server_port)
        try:
            cdp.connect(timeout=2)
            cdp.navigate('http://127.0.0.1/benchmark.html')
            self.assertTrue(cdp.ready(timeout=1))
            cdp.scroll(200, 250, 400)
            cdp.text('a')
            self.assertEqual(cdp.metrics(), {'frames':31})
            self.assertIn('Input.dispatchMouseEvent', methods)
            self.assertIn('Input.insertText', methods)
        finally:
            cdp.close()
            http.shutdown()
            http.server_close()
            ws.close()
            http_thread.join(timeout=2)
            ws_thread.join(timeout=2)

if __name__ == '__main__':
    unittest.main()
