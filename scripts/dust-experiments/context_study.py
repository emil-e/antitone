#!/usr/bin/env python3
"""Inspect polarity/shape/context veto tradeoffs; not an accepted dust classifier."""

import argparse
import html
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

import experiment as e


def context_filter(camera, mask, coherence_limit=None, min_compactness=0.35, max_aspect=4):
    gray = cv2.GaussianBlur(e.analysis_gray(camera), (0, 0), 2)
    gx = cv2.Sobel(gray, cv2.CV_32F, 1, 0) / 8
    gy = cv2.Sobel(gray, cv2.CV_32F, 0, 1) / 8
    count, labels, stats, _ = cv2.connectedComponentsWithStats(
        mask.astype(np.uint8), connectivity=8
    )
    accepted = np.zeros(count, bool)
    descriptors = []
    for i in range(1, count):
        x, y, w, h, area = map(int, stats[i])
        compactness = area / (w * h)
        aspect = max(w, h) / min(w, h)
        x0, y0 = max(0, x - 20), max(0, y - 20)
        x1, y1 = min(mask.shape[1], x + w + 20), min(mask.shape[0], y + h + 20)
        local = (labels[y0:y1, x0:x1] == i).astype(np.uint8)
        outer = cv2.dilate(local, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (41, 41))) > 0
        inner = cv2.dilate(local, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (13, 13))) > 0
        ring = outer & ~inner
        a, b = gx[y0:y1, x0:x1][ring], gy[y0:y1, x0:x1][ring]
        xx, yy, xy = float(np.sum(a * a)), float(np.sum(b * b)), float(np.sum(a * b))
        coherence = float(np.hypot(xx - yy, 2 * xy) / max(xx + yy, 1e-30))
        accepted[i] = compactness >= min_compactness and aspect <= max_aspect
        if coherence_limit is not None:
            accepted[i] &= coherence <= coherence_limit
        descriptors.append(
            {
                "component": i,
                "bbox": [x, y, w, h],
                "area": area,
                "compactness": compactness,
                "aspect": aspect,
                "context_coherence": coherence,
                "accepted": bool(accepted[i]),
            }
        )
    return accepted[labels], descriptors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crops", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in json.loads(args.cases.read_text()):
        crop = args.crops / case["crop"]
        _, camera, _ = e.load_crop(crop)
        support = e.mask_from_shapes(camera.shape, case["shapes"])
        if not support.any():
            raise ValueError("Empty annotation")
        gray = e.analysis_gray(camera)
        noise = e.noise_map(gray)
        absolute = (
            np.maximum.reduce(
                [np.abs(gray - cv2.GaussianBlur(gray, (0, 0), s)) for s in (2, 5, 12)]
            )
            / noise
        )
        positive = e.positive_gaussian_score(camera)
        original = Image.open(crop / "inspection-plus2.png").convert("RGB")
        panels = [("original", original)]
        for threshold in (6, 10):
            raw = e.candidate_mask(positive, threshold)
            shape, features = context_filter(camera, raw)
            context05, features05 = context_filter(camera, raw, coherence_limit=0.5)
            context08, features08 = context_filter(camera, raw, coherence_limit=0.8)
            masks = [
                ("absolute", e.candidate_mask(absolute, threshold), []),
                ("positive", raw, []),
                ("positive-shape", shape, features),
                ("context-0.5", context05, features05),
                ("context-0.8", context08, features08),
            ]
            for name, mask, descriptors in masks:
                grown = (
                    cv2.dilate(
                        mask.astype(np.uint8), cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (9, 9))
                    )
                    > 0
                )
                rows.append(
                    {
                        "case": case["name"],
                        "kind": case["kind"],
                        "method": name,
                        "threshold": threshold,
                        "annotation_coverage": int((grown & support).sum()) / int(support.sum()),
                        "outside_annotation_flagged_pixels": int((grown & ~support).sum()),
                        "descriptors": descriptors,
                    }
                )
                if threshold == 10:
                    panels.append((name + " 10,+4px", e.overlay(np.asarray(original), grown)))
        filename = case["name"] + ".png"
        e.make_sheet(panels, args.output / filename, 256)
        escaped = html.escape(filename, quote=True)
        sections.append(
            f"<h2>{html.escape(case['name'])} — {html.escape(case['kind'])}</h2>"
            f"<p>{html.escape(case.get('notes', ''))}</p><a href='{escaped}'><img src='{escaped}'></a>"
        )
    (args.output / "measurements.json").write_text(json.dumps(rows, indent=2))
    (args.output / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Context veto study</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:100%}a{color:#8cf}</style>"
        "<h1>Polarity, shape and context veto tradeoffs</h1><p>Fixed local Gaussian scores 6/10. Shape requires "
        "component compactness >=0.35 and aspect <=4. Context veto additionally rejects structure-tensor "
        "coherence >0.5 or >0.8 in a 6..20-pixel annulus; these are exploratory cutoffs, not fitted probabilities. "
        "All overlays use four-pixel growth after filtering. Dust near a legitimate edge can be rejected too.</p>"
        "<p>Annotations are incomplete and mostly proposal-selected. Coverage is repair-support overlap, not recall. "
        "Pixels flagged outside annotations include unlabelled dust and are NOT measured false positives. "
        "Held-out/ambiguous features must not be counted as real dust labels.</p>"
        + "".join(sections)
    )
    print(f"{len(rows)} context-veto annotation comparisons")


if __name__ == "__main__":
    main()
