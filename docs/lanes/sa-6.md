# SA-6 "sam3-plant" — status

SAM 3 open weights on plant's RTX 3060 for the A2 recorder
(Muse Spark still judges), plus `--mask faces,license plates`.

## Done

- `ops/open-weights/sam3_boxes.py` (new): local PCS runs
  (`facebook/sam3` via transformers `Sam3Model`/`Sam3Processor`,
  CUDA device map), one process per sample, emitting the same box JSON
  shape as the hosted SAM 3.1 call
  (`label, object_id, x, y, w, h, score`; fractions, 4dp, range-checked
  exactly like `parse_segmentation_line`). `--mask` blackens the face /
  license-plate boxes into `--masked-out`; latency + peak VRAM +
  revision print to stderr for the operator to paste back.
- `xtask/ops/a2-record.py`: `--sam-backend hosted|open-weights`
  (default hosted, behaviour unchanged), `--sam3-script` (defaults to
  `ops/open-weights/sam3_boxes.py`), `--mask` (open-weights only;
  hosted+mask aborts). Masked pixels feed Spark; records carry
  `sam_backend` + `masked`; open-weights records list
  `facebook/sam3` in `models`.
- `ops/open-weights/tests/test_sam3_boxes.py`: 13 stdlib-unittest tests
  green (`python3 ops/open-weights/tests/test_sam3_boxes.py`) —
  adapter/hosted shape parity, degenerate-box parity, mask-target
  parsing + aliases + rejection, region blackening, recorder wiring.
- README "Models" row for `facebook/sam3` added.
- Model-gate (`xtask/src/main.rs` `MODEL_NEEDLES`, deny-list): no edit
  needed — `facebook/sam3`, `huggingface` match no needle; gate run
  green (see below). Nothing but a Meta model runs at runtime.

## Blocked — HF licence/token (operator item 3)

Plant state (presence only, nothing printed): no token file under
`~/.cache/huggingface/` or `~/.huggingface/`, `HF_TOKEN` /
`HUGGING_FACE_HUB_TOKEN` unset, no `~/sam3-venv` (default python3 has
no torch). `facebook/sam3` is access-gated (licence "other": accept the
Meta terms on the model page first — the raw model-card fetch returns
401 until then), so weights, latency, peak VRAM and the pinned revision
are unmeasured. Everything up to the download is implemented.

Exact gated command (operator, on plant, venv only — never root):

```sh
ssh plant
python3 -m venv ~/sam3-venv
~/sam3-venv/bin/pip install -U torch transformers accelerate pillow huggingface_hub
# 1. accept the licence at https://huggingface.co/facebook/sam3
~/sam3-venv/bin/huggingface-cli login   # token prompted, never in repo/logs
~/sam3-venv/bin/huggingface-cli download facebook/sam3   # record the revision digest
~/sam3-venv/bin/python <repo>/ops/open-weights/sam3_boxes.py \
  --image web/static/demo/syn-doc-clean.png \
  --prompt "document page" --prompt "official seal" \
  --prompt "embossed stamp" --prompt "face" \
  --out /tmp/sam3-boxes.json --revision <digest-from-download> \
  --mask faces,license plates --masked-out /tmp/sam3-masked.png
```

Then (staging key sourced in the remote shell only):

```sh
META_API_KEY=... python3 xtask/ops/a2-record.py --png-dir web/static/demo \
  --out-dir /tmp/a2-open --sam-backend open-weights --mask "faces,license plates"
```

## Follow-ups (not SA-6)

- `xtask/ops/ops-a2-record.sh` step 2 asserts `sam-3.1` in `models`;
  open-weights records list `facebook/sam3`, so its owner needs to
  accept either id before an open-weights recording can install.
- Weights revision + licence text + latency/VRAM numbers land here
  once the gate above is run.
