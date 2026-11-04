#!/usr/bin/env python3
"""SAM 3 open weights: local box proposals for the A2 recorder (plant).

Runs Meta's SAM 3 (``facebook/sam3``, gated Meta licence, Promptable
Concept Segmentation) on plant's RTX 3060 via the home venv's
CUDA-bundled PyTorch + transformers, and emits the SAME box JSON shape
as the hosted SAM 3.1 call in ``xtask a2-record`` (``xtask/src/a2_record.rs``)::

    [{"label": ..., "object_id": ..., "x": ..., "y": ...,
      "w": ..., "h": ..., "score": ...}]

with x/y/w/h as fractions of image width/height. ``xtask a2-record
--sam-backend open-weights --sam3-python ~/sam3-venv/bin/python`` runs
this script once per sample; Muse Spark still judges every verdict over
the hosted API.

``--mask faces,license plates`` blackens the detected face /
license-plate boxes in a copy of the input (privacy redaction before
anything is judged or stored) and writes it to ``--masked-out``.

Needs (plant home venv, never root)::

    ~/sam3-venv/bin/pip install -U torch transformers accelerate pillow huggingface_hub
    # accept the gated licence at https://huggingface.co/facebook/sam3, then:
    ~/sam3-venv/bin/huggingface-cli login        # token prompted, never stored in repo/logs
    ~/sam3-venv/bin/huggingface-cli download facebook/sam3

Usage::

    ~/sam3-venv/bin/python sam3_boxes.py --image web/static/demo/syn-doc-clean.png \\
        --prompt "document page" --prompt "face" --out /tmp/sam3-boxes.json \\
        --mask faces,license plates --masked-out /tmp/masked.png

Module import stays stdlib-only (torch/transformers/PIL import lazily
inside the runners) so ``ops/open-weights/tests`` can check the
box-shape adapter on machines without the weights.
"""

import argparse
import json
import sys
import time

MODEL_ID = "facebook/sam3"
MODEL_LICENCE = "gated Meta licence (accept at https://huggingface.co/facebook/sam3)"
# Same bare noun phrases as SAM_PROMPTS in xtask/src/a2_record.rs: SAM 3
# takes short concept phrases, one per request.
DEFAULT_PROMPTS = ["document page", "official seal", "embossed stamp", "face"]

# --mask target -> SAM prompt(s) that locate it.
MASK_PROMPTS = {
    "face": ("face",),
    "license_plate": ("license plate",),
}
MASK_ALIASES = {
    "face": "face",
    "faces": "face",
    "license plate": "license_plate",
    "license plates": "license_plate",
    "license-plate": "license_plate",
    "license-plates": "license_plate",
    "plate": "license_plate",
    "plates": "license_plate",
}


def parse_mask_targets(spec):
    """'faces,license plates' -> ['face', 'license_plate'].

    Raises ValueError naming the valid targets on unknown input.
    """
    targets = []
    for raw in (spec or "").split(","):
        name = " ".join(raw.strip().lower().split())
        if not name:
            continue
        if name not in MASK_ALIASES:
            raise ValueError(
                f"unknown mask target {raw.strip()!r} "
                f"(valid: face, license_plate aliases 'faces', 'license plates')"
            )
        target = MASK_ALIASES[name]
        if target not in targets:
            targets.append(target)
    return targets


def boxes_to_records(label, boxes_xyxy, scores, img_w, img_h):
    """Pixel boxes -> hosted-shape records (fractions, 4dp, range-checked).

    Mirrors PrecheckBox::from_sam() in xtask/src/a2_record.rs: the same
    six keys plus score, x/y inside 0..1, w/h positive. object_id is the
    per-prompt detection index (hosted ids are model-assigned; only the
    shape is contractual, never the values).
    """
    records = []
    for object_id, (box, score) in enumerate(zip(boxes_xyxy, scores)):
        x1, y1, x2, y2 = (float(v) for v in box)
        if x2 < x1 or y2 < y1:
            continue
        rec = {
            "label": label,
            "object_id": object_id,
            "x": round(x1 / img_w, 4),
            "y": round(y1 / img_h, 4),
            "w": round((x2 - x1) / img_w, 4),
            "h": round((y2 - y1) / img_h, 4),
            "score": float(score),
        }
        if 0 <= rec["x"] <= 1 and 0 <= rec["y"] <= 1 and rec["w"] > 0 and rec["h"] > 0:
            records.append(rec)
    return records


def records_for_masking(boxes, mask_targets):
    """Boxes that --mask must blacken -> [(x1,y1,x2,y2) pixel floats]."""
    wanted = set()
    for target in mask_targets:
        wanted.update(MASK_PROMPTS[target])
    regions = []
    for box in boxes:
        if box["label"] in wanted and box["w"] > 0 and box["h"] > 0:
            regions.append(box)
    return regions


def blacken_regions(pixels, img_w, img_h, regions_px):
    """Set RGB bytes inside each int pixel box to zero. Returns box count.

    ``pixels`` is a row-major RGB bytearray (len == img_w*img_h*3);
    ``regions_px`` holds (x1, y1, x2, y2) ints, clamped to the frame.
    Pure stdlib so the tests cover it without PIL.
    """
    count = 0
    for x1, y1, x2, y2 in regions_px:
        x1 = max(0, min(img_w, x1))
        y1 = max(0, min(img_h, y1))
        x2 = max(0, min(img_w, x2))
        y2 = max(0, min(img_h, y2))
        if x2 <= x1 or y2 <= y1:
            continue
        for y in range(y1, y2):
            base = (y * img_w + x1) * 3
            pixels[base:base + (x2 - x1) * 3] = b"\x00" * ((x2 - x1) * 3)
        count += 1
    return count


def run_local(prompts, image_path, threshold, device, revision):
    """One process, one model load: every prompt -> hosted-shape boxes.

    Returns (boxes, counts, seconds, vram_peak_mb, img_w, img_h, pil_image).
    Heavy imports live here so plain `import sam3_boxes` needs nothing.
    """
    import torch
    from PIL import Image
    from transformers import Sam3Model, Sam3Processor

    started = time.monotonic()
    image = Image.open(image_path).convert("RGB")
    img_w, img_h = image.size
    model = Sam3Model.from_pretrained(MODEL_ID, revision=revision, device_map="auto")
    processor = Sam3Processor.from_pretrained(MODEL_ID, revision=revision)
    if device == "cpu":
        model = model.to("cpu")
    model.eval()

    boxes, counts = [], {}
    with torch.no_grad():
        for prompt in prompts:
            inputs = processor(images=image, text=prompt, return_tensors="pt").to(model.device)
            outputs = model(**inputs)
            results = processor.post_process_instance_segmentation(
                outputs,
                threshold=threshold,
                mask_threshold=threshold,
                target_sizes=inputs.get("original_sizes").tolist(),
            )[0]
            found = boxes_to_records(
                prompt,
                results["boxes"].tolist(),
                results["scores"].tolist(),
                img_w,
                img_h,
            )
            boxes.extend(found)
            counts[prompt] = len(found)
    secs = time.monotonic() - started
    peak_mb = None
    if torch.cuda.is_available():
        peak_mb = round(torch.cuda.max_memory_allocated() / (1024 * 1024))
    return boxes, counts, secs, peak_mb, img_w, img_h, image


def save_masked(image, boxes, mask_targets, masked_out):
    """Blacken mask-target boxes in a copy; write PNG. Returns region count."""
    from PIL import Image

    img_w, img_h = image.size
    raw = bytearray(image.tobytes())
    regions = []
    for box in records_for_masking(boxes, mask_targets):
        regions.append((
            int(box["x"] * img_w),
            int(box["y"] * img_h),
            int((box["x"] + box["w"]) * img_w),
            int((box["y"] + box["h"]) * img_h),
        ))
    count = blacken_regions(raw, img_w, img_h, regions)
    Image.frombytes("RGB", (img_w, img_h), bytes(raw)).save(masked_out)
    return count


def build_parser():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--image", required=True, help="input PNG/JPEG")
    ap.add_argument("--prompt", action="append", default=[],
                    help="concept noun phrase (repeatable; defaults mirror xtask a2-record)")
    ap.add_argument("--out", required=True, help="write hosted-shape boxes JSON array here")
    ap.add_argument("--mask", default="",
                    help='comma list to blacken, e.g. "faces,license plates"')
    ap.add_argument("--masked-out", default="",
                    help="masked PNG destination (required with --mask)")
    ap.add_argument("--threshold", type=float, default=0.5)
    ap.add_argument("--device", default="auto", choices=["auto", "cuda", "cpu"])
    ap.add_argument("--revision", default=None,
                    help="weights revision pin (record the digest from the download)")
    return ap


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        mask_targets = parse_mask_targets(args.mask)
    except ValueError as e:
        print(f"ABORT: {e}", file=sys.stderr)
        return 2
    if mask_targets and not args.masked_out:
        print("ABORT: --mask needs --masked-out <png>", file=sys.stderr)
        return 2
    prompts = args.prompt or list(DEFAULT_PROMPTS)
    if args.device == "cuda":
        pass  # device_map="auto" already prefers CUDA; kept for the cpu escape hatch

    boxes, counts, secs, peak_mb, _w, _h, image = run_local(
        prompts, args.image, args.threshold, args.device, args.revision)
    with open(args.out, "w") as f:
        json.dump(boxes, f, indent=2)
        f.write("\n")
    masked = 0
    if mask_targets:
        masked = save_masked(image, boxes, mask_targets, args.masked_out)
    print(f"sam3: {len(boxes)} box(es) {counts} in {secs:.1f}s "
          f"vram_peak_mb={peak_mb} revision={args.revision or 'main'} "
          f"model={MODEL_ID} masked={masked}", file=sys.stderr)
    print(f"wrote {args.out}" + (f" + {args.masked_out}" if mask_targets else ""),
          file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
