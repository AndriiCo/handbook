#!/usr/bin/env python3
"""Record A2 demo-precheck verdicts from live Meta models (operator-run).

Reads META_API_KEY from the environment (never in repo/logs). For each
syn-doc-*.png in --png-dir: SAM 3.1 reads the pixels for boxes, then
Muse Spark judges pass/retake from the sample description plus the SAM
boxes (text-only chat shape, pinned in docs/meta-api-shapes.md). Writes
one <sample>.json per sample to --out-dir; the web tier serves those
files verbatim as replay (zero marginal model cost).

SAM request shape: POST <base>/v1/responses with model sam-3.1 and
`input` blocks (pinned in docs/meta-api-shapes.md); boxes parsed from
`output[]` items' `output_text`, plus `<|box;...|>` tokens and
boxes/masks arrays when present. Non-2xx aborts loudly (no fallback
shape: undocumented endpoints stay unsent).

Usage:
  META_API_KEY=... python3 a2-record.py --png-dir web/static/demo --out-dir /tmp/a2-out
"""

import argparse
import base64
import json
import re
import struct
import sys
import time
import urllib.request

SAM_MODEL = "sam-3.1"
SPARK_MODEL = "muse-spark-1.3"
SAM_PROMPTS = ["document page", "white rectangular page", "official seal",
               "blue circular seal stamp", "embossed stamp", "face"]


def post_json(url, key, body, timeout=120):
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            payload = res.read()
            return res.status, json.loads(payload), time.monotonic() - started
    except urllib.error.HTTPError as e:
        return e.code, e.read()[:500].decode("utf-8", "replace"), time.monotonic() - started


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(26)
    assert head[:8] == b"\x89PNG\r\n\x1a\n", f"{path}: not a PNG"
    return struct.unpack(">II", head[16:24])


def parse_box_tokens(text):
    """<|box;x;y;w;h[;label]|> tokens -> [{label,x,y,w,h}]."""
    boxes = []
    for m in re.finditer(r"<\|box;([^|]*)\|>", text or ""):
        parts = [p.strip() for p in re.split(r"[;,]", m.group(1))]
        nums = []
        label = ""
        for p in parts:
            try:
                nums.append(float(p))
            except ValueError:
                if p and not label:
                    label = p
        if len(nums) >= 4:
            boxes.append({"label": label or "box", "x": nums[0], "y": nums[1],
                          "w": nums[2], "h": nums[3]})
    return boxes


def normalize_boxes(boxes, w, h):
    out = []
    for b in boxes:
        vals = [b["x"], b["y"], b["w"], b["h"]]
        if any(v > 1 for v in vals):
            vals = [b["x"] / w, b["y"] / h, b["w"] / w, b["h"] / h]
        out.append({"label": b["label"], "x": round(vals[0], 4), "y": round(vals[1], 4),
                    "w": round(vals[2], 4), "h": round(vals[3], 4)})
    return [b for b in out if 0 <= b["x"] <= 1 and 0 <= b["y"] <= 1 and b["w"] > 0 and b["h"] > 0]


def sam_boxes(base, key, image_b64, dims):
    body = {
        "model": SAM_MODEL,
        "input": [{
            "role": "user",
            "content": [
                {"type": "input_text",
                 "text": "Locate: " + ", ".join(SAM_PROMPTS)},
                {"type": "input_image",
                 "image_url": "data:image/png;base64," + image_b64},
            ],
        }],
    }
    status, payload, secs = post_json(base + "/v1/responses", key, body)
    if status < 200 or status >= 300:
        raise RuntimeError(f"SAM call failed: HTTP {status}: {payload}")
    if isinstance(payload, dict):
        text = "\n".join(
            block.get("text", "")
            for item in payload.get("output", [])
            if isinstance(item, dict)
            for block in item.get("content", [])
            if isinstance(block, dict) and block.get("type") == "output_text"
        )
        boxes = parse_box_tokens(text or json.dumps(payload))
        arr = payload.get("boxes") or payload.get("masks") or []
        for item in arr if isinstance(arr, list) else []:
            try:
                boxes.append({"label": str(item.get("label", "box")),
                              "x": float(item.get("x", 0)), "y": float(item.get("y", 0)),
                              "w": float(item.get("w", 0)), "h": float(item.get("h", 0))})
            except (ValueError, TypeError, AttributeError):
                continue
        return normalize_boxes(boxes, *dims), secs
    raise RuntimeError(f"SAM call failed: unexpected body: {payload}")


def spark_verdict(base, key, sample_id, label, boxes, image_b64):
    schema = {
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["pass", "retake"]},
            "issues": {"type": "array", "items": {"type": "string"}},
        },
        "required": ["verdict", "issues"],
        "additionalProperties": False,
    }
    # Spark sees the pixels itself (vision): SAM boxes are evidence, but
    # flat synthetic shapes ground to zero boxes, so pixels decide.
    user_text = (
        f"Sample {sample_id} ({label}). SAM 3.1 found these boxes "
        f"(fractions of image width/height): {json.dumps(boxes)}. "
        "Judge: pass only when a document page is fully visible with a "
        "seal-like mark present and no crop/darkness defect. Reply with "
        "verdict + short issues[] (empty when pass)."
    )
    body = {
        "model": SPARK_MODEL,
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": "You judge document-photo prechecks from the image plus SAM box evidence. Reply with the schema only. " + user_text},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64," + image_b64}},
            ]},
        ],
        "temperature": 0,
        "max_completion_tokens": 4000,
        "response_format": {"type": "json_schema",
                            "json_schema": {"name": "PrecheckVerdict", "schema": schema}},
        "prompt_cache_key": "demo-precheck-v1",
        "stream": False,
    }
    status, payload, secs = post_json(base + "/v1/chat/completions", key, body)
    if status < 200 or status >= 300:
        raise RuntimeError(f"Spark call failed: HTTP {status}: {payload}")
    try:
        choice = payload["choices"][0]
        content = choice["message"]["content"]
        verdict = json.loads(content)
        assert verdict["verdict"] in ("pass", "retake")
        assert isinstance(verdict["issues"], list)
    except (KeyError, IndexError, AssertionError, ValueError, TypeError) as e:
        detail = choice.get("finish_reason") if "choice" in dir() else "?"
        raise RuntimeError(f"Spark call failed: bad verdict shape: {e} (finish={detail})")
    usage = payload.get("usage", {}) or {}
    return verdict, usage, secs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--png-dir", required=True)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--base", default="https://api.meta.ai")
    args = ap.parse_args()

    import os
    key = os.environ.get("META_API_KEY", "")
    if not key:
        print("ABORT: META_API_KEY is not set (export it in this shell only)", file=sys.stderr)
        return 1

    import pathlib
    pngs = sorted(pathlib.Path(args.png_dir).glob("syn-doc-*.png"))
    if not pngs:
        print(f"ABORT: no syn-doc-*.png in {args.png_dir}", file=sys.stderr)
        return 1
    out = pathlib.Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)

    total_in, total_out = 0, 0
    for png in pngs:
        sample = png.stem
        print(f"== {sample}", flush=True)
        dims = png_size(png)
        image_b64 = base64.b64encode(png.read_bytes()).decode()
        boxes, sam_secs = sam_boxes(args.base, key, image_b64, dims)
        print(f"  sam: {len(boxes)} box(es) in {sam_secs:.1f}s", flush=True)
        verdict, usage, spark_secs = spark_verdict(args.base, key, sample, sample.replace("-", " "), boxes, image_b64)
        total_in += int(usage.get("prompt_tokens", 0) or 0)
        total_out += int(usage.get("completion_tokens", 0) or 0)
        print(f"  spark: {verdict['verdict']} {verdict['issues']} in {spark_secs:.1f}s "
              f"(in={usage.get('prompt_tokens', '?')} out={usage.get('completion_tokens', '?')})", flush=True)
        record = {
            "verdict": verdict["verdict"],
            "issues": [str(i) for i in verdict["issues"]][:8],
            "boxes": boxes,
            "models": [SAM_MODEL, SPARK_MODEL],
            "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        dest = out / f"{sample}.json"
        dest.write_text(json.dumps(record, indent=2) + "\n")
        print(f"  wrote {dest}", flush=True)
    print(f"DONE: {len(pngs)} recording(s) tokens in={total_in} out={total_out} — paste this whole output back to the agent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
