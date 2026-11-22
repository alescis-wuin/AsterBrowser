"""Small tests for the log comparator, independent of Chromium and Slint."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

MODULE = Path(__file__).with_name('compare-loop.py')
spec = importlib.util.spec_from_file_location('compare_loop', MODULE)
compare_loop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare_loop)


class LogComparatorTest(unittest.TestCase):
    def test_parses_core_metrics_and_does_not_infer_close_without_ack(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'test.log'
            path.write_text('''
[aster:display] requested=Wayland cef_ozone=wayland wayland_socket=true display=false native_wayland_candidate=true
[aster:slint] ticks=7.0/s frames_delivered=0.0/s
[aster:pump] checks=9.0/s calls=8.0/s scheduled=10 fallback=6 skipped=2 requests=13 pump_avg=0.14ms pump_max=1.20ms
[aster:scheduler] mode=event wake_requested=18 wake_posted=8 wake_dispatched=8 wake_coalesced=10 ticks_per_sec=7.0
[aster:gpu] first_valid_frame tab=1
[aster:pump] checks=8.0/s calls=8.0/s scheduled=16 fallback=0 skipped=0 requests=32 pump_avg=0.20ms pump_max=2.00ms wake_avoided=18 wake_issued=14 deadline_late_p50_ms=0.15 deadline_late_p95_ms=1.25 deadline_late_p99_ms=2.00 request_to_pump_p50_ms=1.20 request_to_pump_p95_ms=8.00 request_to_pump_p99_ms=9.00
[aster:latency] input_to_ui_samples=4 input_to_ui_p50_ms=5.00 input_to_ui_p95_ms=18.00 input_to_ui_p99_ms=18.00 gpu_submit_to_ui_samples=10 gpu_submit_to_ui_p50_ms=0.12 gpu_submit_to_ui_p95_ms=0.72 gpu_submit_to_ui_p99_ms=1.00
[aster:load] process_cpu_pct=18.50 process_cpu_ms=370.00 window_ms=2000.00
''', encoding='utf-8')
            result = compare_loop.read_metrics(path)
            self.assertEqual(result['wayland'], 'Wayland / ozone=wayland / socket=true')
            self.assertFalse(result['closing'])
            self.assertTrue(result['first_gpu_frame'])
            self.assertEqual(result['metrics']['pump_calls_per_s'], 8.0)
            self.assertEqual(result['metrics']['wake_coalesced_per_window'], 10.0)
            self.assertEqual(result['metrics']['wake_avoided_per_window'], 18.0)
            self.assertEqual(result['metrics']['deadline_late_p95_ms'], 1.25)
            self.assertEqual(result['metrics']['input_to_ui_p95_ms'], 18.0)
            self.assertEqual(result['metrics']['gpu_submit_to_ui_p95_ms'], 0.72)
            self.assertEqual(result['metrics']['process_cpu_pct'], 18.5)
            path.write_text(path.read_text() + '[aster:lifecycle] cef_shutdown_complete browsers_pending=0\n')
            self.assertTrue(compare_loop.read_metrics(path)['closing'])

    def test_missing_file_is_not_a_zero_metric(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                compare_loop.read_metrics(Path(directory) / 'missing.log')


if __name__ == '__main__':
    unittest.main()
