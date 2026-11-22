import unittest
from aster_bench.plan import best_driver, matrix, variants


class MatrixTests(unittest.TestCase):
    def test_matrix_deterministic_and_balanced(self):
        a = matrix("gpu", ["host-x11"], ["idle", "animation"], 3, 481)
        b = matrix("gpu", ["host-x11"], ["idle", "animation"], 3, 481)
        self.assertEqual([c.name for c in a], [c.name for c in b])
        self.assertEqual(len(a), 36)
        self.assertEqual(len(set(c.name for c in a)), len(a))
        self.assertEqual({c.variant.buffers for c in a}, {1, 2, 3})
        self.assertEqual({c.variant.damage for c in a}, {"legacy", "adaptive"})
        self.assertEqual({c.repetition for c in a}, {1, 2, 3})

    def test_input_drivers_have_explicit_capability_boundary(self):
        self.assertEqual(best_driver("scroll"), "internal")
        self.assertEqual(best_driver("tabs"), "internal")
        self.assertEqual(best_driver("animation"), "cdp")
        with self.assertRaises(ValueError):
            best_driver("resize", "cdp")
        self.assertEqual(best_driver("scroll", "cdp"), "cdp")

    def test_exhaustive_matrix_only_contains_meaningful_combinations(self):
        combinations = variants("cartesian")
        self.assertEqual(len(combinations), 30)
        self.assertEqual(sum(v.pump == "legacy" for v in combinations), 6)
        self.assertEqual(sum(v.presentation == "notify" for v in combinations), 6)
        self.assertEqual(sum(v.heartbeat_ms == 250 for v in combinations), 6)

if __name__ == "__main__":
    unittest.main()
