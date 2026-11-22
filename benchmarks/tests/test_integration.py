"""Exercise real runner orchestration against a fake Aster socket/process.

This is intentionally NOT a fake CEF GPU test. It validates the control
protocol, process-tree sampling, warmup markers, orderly shutdown and rejection
of trials missing GPU evidence. CEF/WGPU hardware remains an external test.
"""
import json
import os
from pathlib import Path
import tempfile
import unittest

from aster_bench.display import Display
from aster_bench.fixture import fixture_server
from aster_bench.plan import matrix
from aster_bench.runner import run_case


FAKE_ASTER = r'''#!/usr/bin/env python3
import json, os, socket, subprocess, sys
port = int(os.environ['ASTER_BENCH_PORT'])
token = os.environ['ASTER_BENCH_TOKEN']
assert 'ASTER_CDP_PORT' not in os.environ, 'native-only tests must not start CDP'
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)', '--type=gpu-process'],
                          stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
snap = {'active':1,'tabs':[{'id':1,'url':'about:blank','title':'Test','loading':False}]}
print('[aster:lifecycle] gpu_renderer_setup device_ready=1 buffers=2', flush=True)
if os.environ.get('FAKE_NO_GPU') != '1':
    print('[aster:gpu] first_valid_frame tab=1', flush=True)
s = socket.socket()
s.bind(('127.0.0.1', port))
s.listen(2)
try:
    connection, _ = s.accept()
    with connection:
        reader = connection.makefile('r', encoding='utf-8')
        for line in reader:
            request = json.loads(line)
            assert request['token'] == token
            action = request['action']
            if action == 'navigate':
                snap['tabs'][0]['url'] = request['url']
            elif action == 'mark':
                print('[aster:benchmark] mark=' + request['label'], flush=True)
                if request['label'] == 'measure_begin':
                    print('[aster:gpu] imported=60.0/s importer_full_copy=95.0MiB/s presentation_writes=6.0MiB/s', flush=True)
                    print('[aster:pump] checks=82.0/s calls=50.0/s scheduled=55 fallback=12 deadline_late_p95_ms=1.50', flush=True)
                    print('[aster:slint] ticks=80.0/s frames_delivered=60.0/s', flush=True)
            elif action == 'quit':
                print('[aster:lifecycle] cef_shutdown_complete browsers_pending=0', flush=True)
            reply = {'id':request['id'],'ok':True,'snapshot':snap}
            connection.sendall((json.dumps(reply)+'\n').encode())
            if action == 'quit':
                break
finally:
    child.terminate()
    child.wait(timeout=3)
    s.close()
'''


class RunnerIntegrationTests(unittest.TestCase):
    def _case(self, no_gpu):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            script = base / 'fake_aster'
            script.write_text(FAKE_ASTER, encoding='utf-8')
            script.chmod(0o755)
            case = matrix('gpu', ['host-x11'], ['idle'], 1, 1, shuffle=False)[0]
            env = os.environ.copy()
            if no_gpu:
                env['FAKE_NO_GPU'] = '1'
            with fixture_server() as fixture:
                return run_case(case, Display('host-x11', env), base, script,
                    fixture, base/'output', warmup=0, duration=1., frequency=5.,
                    driver_preference='internal', seed=2, startup_timeout=3,
                    allow_cpu_fallback=False)

    def test_protocol_and_full_run_with_process_descendants(self):
        result = self._case(no_gpu=False)
        self.assertEqual(result['status'], 'pass', result['reasons'])
        self.assertTrue(result['document_ready'])
        self.assertEqual(result['returncode'], 0)
        self.assertGreater(result['process']['monitor_samples'], 0)
        self.assertGreaterEqual(result['process']['max_chromium_children'], 1)
        self.assertGreaterEqual(result['process']['max_gpu_processes'], 1)
        self.assertEqual(result['telemetry']['averages']['gpu_presentation_writes'], 6.)

    def test_missing_gpu_proof_fails_even_when_process_exits_normally(self):
        result = self._case(no_gpu=True)
        self.assertEqual(result['returncode'], 0)
        self.assertEqual(result['status'], 'fail')
        self.assertIn('no_accelerated_cef_frame', result['reasons'])

if __name__ == '__main__':
    unittest.main()
