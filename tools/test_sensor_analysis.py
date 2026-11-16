import unittest
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

spec = spec_from_file_location("sensor_analysis", Path(__file__).with_name("analyze-sensor-validation.py"))
analysis = module_from_spec(spec)
spec.loader.exec_module(analysis)

class SensorAnalysisTests(unittest.TestCase):
    def test_gpu_load_phase_and_sequential_reference_are_preserved(self):
        rows = []
        for phase, temp in [("baseline", 40), ("gpu_load", 50), ("recovery", 45)]:
            rows.append({"phase": phase, "sample": {"ec_celsius": [65, temp], "actual_rpm": [2000, 2000, 0], "modes": [[0, 0]]*3},
                         "nvidia": {"celsius": temp, "utilization_percent": 25}, "utc_unix_ms": 1060, "thermal_captured_unix_ms": 1000})
        report = analysis.gpu_comparison(rows)
        self.assertEqual(report["phases"]["gpu_load"]["samples"], 1)
        self.assertEqual(report["ec_b_vs_nvidia"]["mean_absolute_difference_c"], 0)
        self.assertEqual(report["ec_b_vs_nvidia"]["correlation"], 1)
        self.assertEqual(report["max_provider_query_window_ms"], 60)

    def test_empty_reference_does_not_manufacture_agreement(self):
        report = analysis.gpu_comparison([])
        self.assertNotIn("ec_b_vs_nvidia", report)
        self.assertEqual(report["phases"]["gpu_load"]["samples"], 0)

if __name__ == "__main__":
    unittest.main()
