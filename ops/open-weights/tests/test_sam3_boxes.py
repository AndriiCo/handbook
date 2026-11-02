#!/usr/bin/env python3
"""Box-shape adapter tests for ops/open-weights/sam3_boxes.py (stdlib only).

Run from the repo root::

    python3 ops/open-weights/tests/test_sam3_boxes.py

Covers: the open-weights adapter emits the same box record shape as the
hosted SAM 3.1 parser in xtask/ops/a2-record.py; --mask target parsing
(incl. aliases and rejection); region blackening on a synthetic buffer.
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve()
OPEN_WEIGHTS = HERE.parent.parent  # ops/open-weights
REPO_ROOT = OPEN_WEIGHTS.parent.parent
sys.path.insert(0, str(OPEN_WEIGHTS))


def load_dashed(path):
    spec = importlib.util.spec_from_file_location("a2_record_dashed", str(path))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


import sam3_boxes  # noqa: E402  (path fixed above)


A2 = load_dashed(REPO_ROOT / "xtask" / "ops" / "a2-record.py")

HOSTED_KEYS = {"label", "object_id", "x", "y", "w", "h", "score"}


class AdapterShapeTest(unittest.TestCase):
    def test_same_keys_as_hosted(self):
        """Every adapter key set == the hosted parser's key set."""
        hosted = A2.parse_segmentation_output(
            "0<|box;x1=10;y1=20;x2=110;y2=220;w=400;h=800;c=0.9|>", "face")
        local = sam3_boxes.boxes_to_records("face", [(10, 20, 110, 220)], [0.9], 400, 800)
        self.assertEqual(len(hosted), 1, hosted)
        self.assertEqual(len(local), 1, local)
        self.assertEqual(set(local[0]), HOSTED_KEYS)
        self.assertEqual(set(hosted[0]), HOSTED_KEYS)
        for key in ("label", "object_id", "x", "y", "w", "h", "score"):
            self.assertEqual(type(local[0][key]), type(hosted[0][key]), key)
        self.assertEqual(local[0]["label"], "face")
        self.assertAlmostEqual(local[0]["x"], hosted[0]["x"])
        self.assertAlmostEqual(local[0]["y"], hosted[0]["y"])
        self.assertAlmostEqual(local[0]["w"], hosted[0]["w"])
        self.assertAlmostEqual(local[0]["h"], hosted[0]["h"])

    def test_fractions_and_ranges(self):
        recs = sam3_boxes.boxes_to_records("document page", [(0, 0, 400, 800)], [1.0], 400, 800)
        self.assertEqual(len(recs), 1)
        rec = recs[0]
        self.assertTrue(0 <= rec["x"] <= 1 and 0 <= rec["y"] <= 1)
        self.assertGreater(rec["w"], 0)
        self.assertGreater(rec["h"], 0)

    def test_degenerate_boxes_dropped_like_hosted(self):
        # Zero-area / inverted boxes never survive on either path.
        local = sam3_boxes.boxes_to_records("face", [(5, 5, 5, 9), (9, 9, 3, 3)], [0.5, 0.5], 100, 100)
        self.assertEqual(local, [])
        hosted = A2.parse_segmentation_output(
            "0<|box;x1=5;y1=5;x2=5;y2=9;w=100;h=100|>", "face")
        self.assertEqual(hosted, [])

    def test_out_file_is_json_array_of_records(self):
        recs = sam3_boxes.boxes_to_records("face", [(10, 20, 30, 40)], [0.7], 100, 100)
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "boxes.json"
            out.write_text(json.dumps(recs, indent=2) + "\n")
            back = json.loads(out.read_text())
        self.assertEqual(len(back), 1)
        self.assertEqual(set(back[0]), HOSTED_KEYS)


class MaskTargetsTest(unittest.TestCase):
    def test_spec_examples(self):
        self.assertEqual(sam3_boxes.parse_mask_targets("faces,license plates"),
                         ["face", "license_plate"])
        self.assertEqual(sam3_boxes.parse_mask_targets("face"), ["face"])
        self.assertEqual(sam3_boxes.parse_mask_targets(""), [])

    def test_aliases(self):
        for alias in ("license-plate", "license-plates", "plate", "plates", "Faces"):
            self.assertIn(sam3_boxes.parse_mask_targets(alias)[0],
                          ("face", "license_plate"), alias)

    def test_unknown_rejected(self):
        with self.assertRaises(ValueError):
            sam3_boxes.parse_mask_targets("faces,passports")

    def test_records_for_masking_selects_prompts(self):
        boxes = [
            {"label": "face", "object_id": 0, "x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2, "score": 0.9},
            {"label": "document page", "object_id": 0, "x": 0.0, "y": 0.0,
             "w": 1.0, "h": 1.0, "score": 0.9},
        ]
        picked = sam3_boxes.records_for_masking(boxes, ["face"])
        self.assertEqual([b["label"] for b in picked], ["face"])


class BlackenTest(unittest.TestCase):
    def test_region_zeroed_rest_untouched(self):
        w, h = 8, 6
        pixels = bytearray([0xAB] * (w * h * 3))
        n = sam3_boxes.blacken_regions(pixels, w, h, [(2, 1, 5, 4)])
        self.assertEqual(n, 1)
        for y in range(h):
            for x in range(w):
                cell = pixels[(y * w + x) * 3:(y * w + x) * 3 + 3]
                inside = 2 <= x < 5 and 1 <= y < 4
                self.assertEqual(bytes(cell), b"\x00\x00\x00" if inside else b"\xab\xab\xab",
                                 (x, y))

    def test_out_of_frame_clamped(self):
        w, h = 4, 4
        pixels = bytearray([0xFF] * (w * h * 3))
        n = sam3_boxes.blacken_regions(pixels, w, h, [(-10, -10, 100, 100)])
        self.assertEqual(n, 1)
        self.assertEqual(bytes(pixels), b"\x00" * (w * h * 3))


class RecorderWiringTest(unittest.TestCase):
    def test_mask_needs_open_weights(self):
        with self.assertRaises(ValueError):
            A2.check_mask_backend("hosted", "faces")
        A2.check_mask_backend("open-weights", "faces")
        A2.check_mask_backend("hosted", "")

    def test_parser_flags(self):
        args = A2.build_parser().parse_args([
            "--png-dir", "a", "--out-dir", "b",
            "--sam-backend", "open-weights",
            "--mask", "faces,license plates"])
        self.assertEqual(args.sam_backend, "open-weights")
        self.assertEqual(args.mask, "faces,license plates")
        defaults = A2.build_parser().parse_args(["--png-dir", "a", "--out-dir", "b"])
        self.assertEqual(defaults.sam_backend, "hosted")
        self.assertEqual(defaults.mask, "")

    def test_default_script_resolves_into_repo(self):
        script = A2.default_sam3_script()
        self.assertTrue(script.endswith("ops/open-weights/sam3_boxes.py"), script)
        self.assertTrue(Path(script).is_file(), script)


if __name__ == "__main__":
    unittest.main()
