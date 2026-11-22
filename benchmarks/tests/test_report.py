import csv
import json
from pathlib import Path
import tempfile
import unittest
from aster_bench.report import write_reports


def case(variant: str, rep: int, cpu: float, log: str = 'test.log'):
    return {'case': f'{variant}-{rep}', 'variant': {'name': variant}, 'scenario': 'animation',
        'display': 'host-x11', 'repetition': rep, 'driver': 'internal',
        'status': 'pass', 'reasons': [], 'actions': 15, 'measurement_seconds': 5,
        'process': {'cpu_total_core_pct': cpu, 'cpu_aster_core_pct': cpu*.5,
                    'cpu_chromium_core_pct': cpu*.5, 'rss_peak_mib': 280},
        'telemetry': {'averages': {'gpu_presentation_writes': 4.5}},
        'dom': {'frames': 30},
        'files': {'log': log, 'events': 'a/events.jsonl', 'process_samples': 'a/tree.csv'}}


class ReportsTests(unittest.TestCase):
    def test_regressions_require_repetitions_and_escape_html(self):
        samples = [case('gpu_legacy_2', 1, 20.), case('gpu_legacy_2', 2, 21.),
                   case('gpu_adaptive_2', 1, 28.), case('gpu_adaptive_2', 2, 29.)]
        samples[0]['reasons'] = ['<script>alert(1)</script>']
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            result = write_reports(output, samples, {'source': 'integration test'})
            self.assertGreater(len(result['regressions']), 0)
            html = (output / 'index.html').read_text()
            self.assertNotIn('<script>alert(1)</script>', html)
            self.assertIn('&lt;script&gt;', html)
            with (output / 'results.csv').open() as src:
                self.assertEqual(len(list(csv.DictReader(src))), 4)
            self.assertEqual(len(json.loads((output/'report.json').read_text())['cases']), 4)

    def test_single_sample_cannot_claim_regression(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = write_reports(Path(tmp), [case('gpu_legacy_2', 1, 1.),
                                                case('gpu_adaptive_2', 1, 200.)], {})
            self.assertEqual(result['regressions'], [])

if __name__ == '__main__':
    unittest.main()
