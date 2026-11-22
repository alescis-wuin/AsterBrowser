import unittest
from aster_bench.telemetry import telemetry, measurement_text, detect_anomalies


class TelemetryTests(unittest.TestCase):
    def test_markers_exclude_warmup_and_preserve_partial_window_metrics(self):
        text = '''[aster:pump] checks=999/s calls=999/s
[aster:benchmark] mark=measure_begin
[aster:pump] checks=40.0/s calls=25.0/s scheduled=11 fallback=3 deadline_late_p95_ms=0.22
[aster:gpu] imported=60.1/s importer_full_copy=96.1MiB/s presentation_writes=6.4MiB/s
[aster:pump] checks=50.0/s calls=30.0/s
[aster:benchmark] mark=measure_end
[aster:pump] checks=999/s calls=999/s
'''
        extracted = measurement_text(text)
        self.assertNotIn('999', extracted)
        values = telemetry(extracted)
        self.assertEqual(values['averages']['pump_calls'], 27.5)
        self.assertEqual(values['window_counts']['gpu_imported'], 1)
        self.assertEqual(values['averages']['gpu_presentation_writes'], 6.4)
        self.assertEqual(values['averages']['pump_deadline_late_p95_ms'], .22)

    def test_empty_latency_windows_are_not_zero_latency_scores(self):
        values = telemetry("[aster:latency] input_to_ui_samples=0 input_to_ui_p95_ms=0.00 "
                           "gpu_submit_to_ui_samples=0 gpu_submit_to_ui_p95_ms=0.00\n"
                           "[aster:latency] input_to_ui_samples=2 input_to_ui_p95_ms=14.0 "
                           "gpu_submit_to_ui_samples=1 gpu_submit_to_ui_p95_ms=2.5")
        self.assertEqual(values['averages']['latency_input_to_ui_p95_ms'], 14.0)
        self.assertEqual(values['averages']['latency_gpu_submit_to_ui_p95_ms'], 2.5)
        self.assertEqual(values['window_counts']['latency_input_to_ui_p95_ms'], 1)

    def test_missing_gpu_frames_and_crashes_never_pass(self):
        log = '[aster:lifecycle] cef_shutdown_complete browsers_pending=0\n'
        self.assertIn('no_accelerated_cef_frame', detect_anomalies(
            log, renderer='gpu', returncode=0, document_ready=True))
        self.assertNotIn('no_accelerated_cef_frame', detect_anomalies(
            log, renderer='cpu', returncode=0, document_ready=True))
        self.assertIn('gpu_fallback_or_capture_failure', detect_anomalies(
            '[aster:gpu] GPU unavailable: fallback\n', renderer='gpu',
            returncode=0, document_ready=True))
        self.assertIn('missing_cef_shutdown_ack', detect_anomalies(
            '', renderer='cpu', returncode=0, document_ready=True))
        self.assertIn('cef_capture_allocation_failed', detect_anomalies(
            'Unable to allocate frame for first frame capture: OOM?', renderer='cpu',
            returncode=0, document_ready=True))

if __name__ == '__main__':
    unittest.main()
