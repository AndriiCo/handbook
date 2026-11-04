#!/usr/bin/env python3
"""Box-shape adapter tests for ops/open-weights/sam3_boxes.py (stdlib only).

Run from the repo root::

    python3 ops/open-weights/tests/test_sam3_boxes.py

Covers: the open-weights adapter emits the same box record shape as the
hosted SAM 3.1 parser in ``xtask a2-record`` (the HOSTED_FACE record below
is pinned by its test ``hosted_boxes_are_fractions_rounded_and_tagged_with_
the_prompt``); --mask target parsing (incl. aliases and rejection); region
blackening on a synthetic buffer.
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve()
OPEN_WEIGHTS = HERE.parent.parent  # ops/open-weights
REPO_ROOT = OPEN_WEIGHTS.parent.parent
sys.path.insert(0, str(OPEN_WEIGHTS))


import sam3_boxes  # noqa: E402  (path fixed above)

HOSTED_KEYS = {"label", "object_id", "x", "y", "w", "h", "score"}

# `xtask a2-record` on the hosted answer
# "0<|box;x1=10;y1=20;x2=110;y2=220;w=400;h=800;c=0.9|>" for "face"
# (pinned on the Rust side by the test named in the module docstring).
HOSTED_FACE = {"label": "face", "object_id": 0, "x": 0.025, "y": 0.025,
               "w": 0.25, "h": 0.25, "score": 0.9}


class AdapterShapeTest(unittest.TestCase):
    def test_same_keys_as_hosted(self):
        """Every adapter key set == the hosted parser's key set."""
        hosted = [HOSTED_FACE]
        local = sam3_boxes.boxes_to_records("face", [(10, 20, 110, 220)], [0.9], 400, 800)
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


if __name__ == "__main__":
    unittest.main()
