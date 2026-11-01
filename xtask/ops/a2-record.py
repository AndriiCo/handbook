#!/usr/bin/env python3
"""Record A2 demo-precheck verdicts from live Meta models (operator-run).

Reads META_API_KEY from the environment (never in repo/logs). For each
syn-doc-*.png in --png-dir: SAM 3.1 reads the pixels for boxes, then
Muse Spark judges pass/retake from the sample description plus the SAM
boxes (text-only chat shape, pinned in docs/meta-api-shapes.md). Writes
one <sample>.json per sample to --out-dir; the web tier serves those
files verbatim as replay (zero marginal model cost).

SAM request shape: POST <base>/v1/responses with model sam-3.1, one
bare noun phrase per request in `input_text` plus the image in
`input_image` (pinned in docs/meta-api-shapes.md). Boxes parse from the
documented token grammar in the `output_text` lane
(`<Nf>id<|box;x1;y1;x2;y2;w;h[;c]|><|mask;...|>,...`), keyed by object
id. Empty output (zero matches) is normal; non-2xx aborts loudly (no
fallback shape: undocumented endpoints stay unsent).

Usage:
  META_API_KEY=... python3 a2-record.py --png-dir web/static/demo --out-dir /tmp/a2-out
"""

import argparse
import base64
import json
import random
import struct
import sys
import time
import urllib.request

SAM_MODEL = "sam-3.1"
SPARK_MODEL = "muse-spark-1.3"
# One bare noun phrase per request. Commands ("locate ..."), questions,
# and comma/and lists return zero matches (sam/segmenting), so each
# concept gets its own call in sam_boxes() below.
SAM_PROMPTS = ["document page", "official seal", "embossed stamp", "face"]


# Retry discipline (error-handling docs): up to 4 attempts with jitter,
# honouring Retry-After on 429/5xx. Terminal statuses (400/401/402/403/
# 404/413) return at once, never retried unchanged.
RETRYABLE = {429, 500, 502, 503, 504}
TERMINAL = {400, 401, 402, 403, 404, 413}
MAX_ATTEMPTS = 4


def retry_after_secs(headers):
    try:
        return max(0, int(str(headers.get("Retry-After", "")).strip()))
    except (ValueError, TypeError, AttributeError):
        return None


def post_json(url, key, body, timeout=120):
    data = json.dumps(body).encode()
    headers = {
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
    }
    delay = 2.0
    for attempt in range(MAX_ATTEMPTS):
        req = urllib.request.Request(url, data=data, headers=headers, method="POST")
        started = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as res:
                payload = res.read()
                return res.status, json.loads(payload), time.monotonic() - started
        except urllib.error.HTTPError as e:
            payload = e.read()[:500].decode("utf-8", "replace")
            secs = time.monotonic() - started
            if e.code in TERMINAL or e.code not in RETRYABLE or attempt == MAX_ATTEMPTS - 1:
                return e.code, payload, secs
            wait = retry_after_secs(e.headers)
            if wait is None:
                wait = delay * (1.0 + random.random() * 0.5)
                delay *= 2
            else:
                wait = min(wait, 60)
            time.sleep(wait)
    raise AssertionError("unreachable: retry loop always returns")


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(26)
    assert head[:8] == b"\x89PNG\r\n\x1a\n", f"{path}: not a PNG"
    return struct.unpack(">II", head[16:24])


def parse_box_fields(inner):
    """Named box-token fields -> dict, or None when required keys miss.

    Parses by key (x1,y1,x2,y2,w,h,c), so tokens read the same with or
    without confidence `c` (sam/reading-segmentation).
    """
    vals = {}
    for field in inner.split(";"):
        if "=" not in field:
            continue
        k, v = field.split("=", 1)
        try:
            vals[k.strip()] = float(v.strip())
        except ValueError:
            continue
    if any(k not in vals for k in ("x1", "y1", "x2", "y2", "w", "h")):
        return None
    if vals["w"] <= 0 or vals["h"] <= 0:
        return None
    if vals["x2"] < vals["x1"] or vals["y2"] < vals["y1"]:
        return None
    return vals


def split_records(line):
    """Frame line -> ["id<|box;..|>..", ...].

    Records anchor on the box-token opener with the object id scanned
    backwards from it: mask payloads are base85 and may hold commas
    and `>`, so never split on those.
    """
    at = []
    cursor = 0
    while True:
        found = line.find("<|box;", cursor)
        if found < 0:
            break
        at.append(found)
        cursor = found + 6

    def record_start(box_pos):
        end = box_pos
        while end > 0 and line[end - 1] in " ,":
            end -= 1
        start = end
        while start > 0 and line[start - 1].isdigit():
            start -= 1
        return start

    records = []
    for i, pos in enumerate(at):
        start = record_start(pos)
        if start == pos:
            continue
        end = record_start(at[i + 1]) if i + 1 < len(at) else len(line)
        if end > start:
            records.append(line[start:end])
    return records


def parse_segmentation_line(line, label):
    """One output_text line -> [{label,object_id,x,y,w,h,score}].

    Box corners are source pixels, normalized by the record's own w/h;
    records key by object id, never by position. An empty lane (zero
    matches) yields no boxes — a normal result, not an error.
    """
    boxes = []
    rest = line.strip()
    if rest.startswith("<"):
        gt = rest.find(">")
        if gt >= 0:
            rest = rest[gt + 1:].lstrip()
    for record in split_records(rest):
        head, sep, tail = record.partition("<|box;")
        if not sep or not head.strip().isdigit():
            continue
        inner = tail.split("|>", 1)[0]
        vals = parse_box_fields(inner)
        if vals is None:
            continue
        w, h = vals["w"], vals["h"]
        boxes.append({
            "label": label,
            "object_id": int(head.strip()),
            "x": round(vals["x1"] / w, 4),
            "y": round(vals["y1"] / h, 4),
            "w": round((vals["x2"] - vals["x1"]) / w, 4),
            "h": round((vals["y2"] - vals["y1"]) / h, 4),
            "score": vals.get("c", 1.0),
        })
    return [b for b in boxes
            if 0 <= b["x"] <= 1 and 0 <= b["y"] <= 1 and b["w"] > 0 and b["h"] > 0]


def parse_segmentation_output(text, label):
    boxes = []
    for line in (text or "").splitlines():
        boxes.extend(parse_segmentation_line(line, label))
    return boxes


def sam_call(base, key, prompt, image_b64):
    """One concept, one request: bare noun phrase + image on /v1/responses."""
    body = {
        "model": SAM_MODEL,
        "input": [{
            "role": "user",
            "content": [
                {"type": "input_text", "text": prompt},
                {"type": "input_image",
                 "image_url": "data:image/png;base64," + image_b64},
            ],
        }],
        "stream": False,
        "metadata": {"include_confidence": "true"},
    }
    status, payload, secs = post_json(base + "/v1/responses", key, body)
    if status < 200 or status >= 300:
        raise RuntimeError(f"SAM call failed for {prompt!r}: HTTP {status}: {payload}")
    if not isinstance(payload, dict):
        raise RuntimeError(f"SAM call failed for {prompt!r}: unexpected body: {payload}")
    if payload.get("status") == "incomplete":
        raise RuntimeError(f"SAM call incomplete for {prompt!r}: {payload.get('incomplete_details')}")
    text = "\n".join(
        block.get("text", "")
        for item in payload.get("output", [])
        if isinstance(item, dict)
        for block in item.get("content", [])
        if isinstance(block, dict) and block.get("type") == "output_text"
    )
    return parse_segmentation_output(text, prompt), secs


def sam_boxes(base, key, image_b64):
    """Segment every SAM_PROMPTS concept; merge matches across requests."""
    boxes, secs, counts = [], 0.0, {}
    for prompt in SAM_PROMPTS:
        found, call_secs = sam_call(base, key, prompt, image_b64)
        boxes.extend(found)
        secs += call_secs
        counts[prompt] = len(found)
    return boxes, secs, counts


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
        "reasoning_effort": "low",
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
    except (KeyError, IndexError, TypeError) as e:
        raise RuntimeError(f"Spark call failed: no choice: {e}: {payload}")
    finish = choice.get("finish_reason")
    if finish == "length":
        raise RuntimeError("Spark call failed: truncated (finish_reason=length) — "
                           "raise max_completion_tokens, never parse the partial JSON")
    message = choice.get("message") or {}
    if message.get("refusal"):
        raise RuntimeError(f"Spark call failed: model refused: {message['refusal']}")
    if finish != "stop":
        raise RuntimeError(f"Spark call failed: finish_reason={finish}")
    try:
        verdict = json.loads(message["content"])
        assert verdict["verdict"] in ("pass", "retake")
        assert isinstance(verdict["issues"], list)
    except (KeyError, AssertionError, ValueError, TypeError) as e:
        raise RuntimeError(f"Spark call failed: bad verdict shape: {e} (finish={finish})")
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
        png_size(png)
        image_b64 = base64.b64encode(png.read_bytes()).decode()
        boxes, sam_secs, counts = sam_boxes(args.base, key, image_b64)
        print(f"  sam: {len(boxes)} box(es) {counts} in {sam_secs:.1f}s", flush=True)
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
