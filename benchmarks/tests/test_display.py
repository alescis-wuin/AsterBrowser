import tempfile
import unittest
from pathlib import Path
from aster_bench.display import _gpu_compositor_log, _xwayland_display


class WestonParsingTests(unittest.TestCase):
    def test_xwayland_display_is_taken_from_weston_output(self):
        self.assertEqual(_xwayland_display('xserver listening on display :5'), ':5')
        self.assertEqual(_xwayland_display('Xwayland launched on display :98'), ':98')
        self.assertIsNone(_xwayland_display('Wayland socket available wayland-7'))

    def test_software_and_unverified_gl_are_not_gpu_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'weston.log'
            path.write_text('GL renderer: llvmpipe')
            self.assertFalse(_gpu_compositor_log(path))
            path.write_text('GL renderer: NVIDIA GeForce RTX')
            self.assertTrue(_gpu_compositor_log(path))
            path.write_text('Weston started')
            self.assertFalse(_gpu_compositor_log(path))

if __name__ == '__main__':
    unittest.main()
