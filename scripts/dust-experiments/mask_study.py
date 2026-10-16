#!/usr/bin/env python3
"""Compare conservative annotations with score-derived repair support."""

import argparse
import html
import json
from pathlib import Path
import time

import cv2
import numpy as np
from PIL import Image

import experiment as e
import stage_study as stages


SCORE_VARIANTS = ((6, 4), (8, 4), (10, 4), (10, 2))


def score_support(annotation, score, threshold, growth, min_area=3):
    selected = ((score >= threshold) & annotation).astype(np.uint8)
    count, labels, stats, _ = cv2.connectedComponentsWithStats(selected, connectivity=8)
    accepted = np.zeros(count, bool)
    for component in range(1, count):
        accepted[component] = stats[component, cv2.CC_STAT_AREA] >= min_area
    core = accepted[labels]
    if not core.any():
        raise ValueError("No high-confidence defect core inside annotation")
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2 * growth + 1, 2 * growth + 1))
    # The supplied editable annotation remains the maximum changed support.
    support = (cv2.dilate(core.astype(np.uint8), kernel) > 0) & annotation
    return core, support


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crops", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--exposure", type=float, default=2)
    args = parser.parse_args()
    cases = json.loads(args.cases.read_text())
    names = [case["name"] for case in cases]
    if len(names) != len(set(names)):
        raise ValueError("Case names must be unique")
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in cases:
        crop = args.crops / case["crop"]
        info, camera, working = e.load_crop(crop)
        annotation = (
            np.asarray(Image.open(case["mask_file"]).convert("L")) > 0
            if "mask_file" in case
            else e.mask_from_shapes(working.shape, case["shapes"])
        )
        if annotation.shape != working.shape[:2] or not annotation.any() or annotation.all():
            raise ValueError("Invalid annotation mask")
        score = e.positive_gaussian_score(camera)
        variants = [("annotation", annotation, None, None, None)]
        for threshold, growth in SCORE_VARIANTS:
            core, support = score_support(annotation, score, threshold, growth)
            variants.append((f"score-{threshold}-grow-{growth}", support, core, threshold, growth))
        for variant in case.get("support_variants", []):
            support = e.mask_from_shapes(working.shape, variant["shapes"])
            if np.any(support & ~annotation):
                raise ValueError("Explicit support exceeds annotation")
            variants.append((variant["name"], support, support, None, None))
        out = args.output / case["name"]
        out.mkdir()
        original = e.render(args.probe, working, out, "original", args.exposure)
        panels = [
            ("original", original.crop(case["zoom"])),
            ("annotation", e.overlay(np.asarray(original), annotation).crop(case["zoom"])),
        ]
        for name, support, core, threshold, growth in variants:
            if not support.any() or support.all():
                raise ValueError("Invalid repair support")
            Image.fromarray(support.astype(np.uint8) * 255).save(out / (name + "-mask.png"))
            if core is not None:
                Image.fromarray(core.astype(np.uint8) * 255).save(out / (name + "-core.png"))
            start = time.perf_counter()
            result, donors = e.repair(working, support, "donor-heal-ranked")
            seconds = time.perf_counter() - start
            image = e.render(args.probe, result, out, name, args.exposure)
            np.save(out / (name + "-masked-values.npy"), result[support])
            stages.donor_diagram(original, support, donors, out / (name + "-donors.png"))
            panels.extend(
                (
                    (name + " mask", e.overlay(np.asarray(original), support).crop(case["zoom"])),
                    (name + " repair", image.crop(case["zoom"])),
                )
            )
            rows.append(
                {
                    "case": case["name"],
                    "kind": case["kind"],
                    "variant": name,
                    "threshold": threshold,
                    "growth": growth,
                    "annotation_pixels": int(annotation.sum()),
                    "core_pixels": None if core is None else int(core.sum()),
                    "support_pixels": int(support.sum()),
                    "support_fraction_of_annotation": float(support.sum() / annotation.sum()),
                    "score_max_in_annotation": float(score[annotation].max()),
                    "donors": donors,
                    "repair_seconds": seconds,
                    "outside_support_bit_exact": True,
                }
            )
        e.make_sheet(panels, out / "detail.png", 256)
        (out / "case.json").write_text(
            json.dumps({"case": case, "source": info, "inspection_ev": args.exposure}, indent=2)
        )
        escaped = html.escape(case["name"], quote=True)
        sections.append(
            f"<h2>{escaped} — {html.escape(case['kind'])}</h2>"
            f"<p>{html.escape(case.get('notes', ''))}</p>"
            f"<a href='{escaped}/detail.png'><img src='{escaped}/detail.png'></a>"
        )
        print(case["name"], flush=True)
    (args.output / "measurements.json").write_text(json.dumps(rows, indent=2))
    (args.output / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Repair support study</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:100%}a{color:#8cf}</style>"
        "<h1>Defect core and repair support</h1><p>Each visual annotation is a maximum editable support. Positive-density "
        "Gaussian score pixels inside it define high-confidence cores; components smaller than three pixels reject. "
        "Elliptical growth remains clipped to the annotation. The frozen boundary-ranked donor healer receives only "
        "the resulting binary support, exactly as a manual mask would. It never receives the score or annotation "
        "origin. Explicit variants are inspection masks, not hidden-truth optimization.</p>"
        "<p>These marks have no clean recaptures. Smaller support changes fewer known-looking pixels but may leave a "
        "low-contrast defect halo; visual inspection is required. Score thresholds are exploratory and scores are not "
        "probabilities. Every repair is finite and preserves f32 bits outside its displayed support. Diagnostic colour "
        "and +2 EV viewing are not photographic acceptance. Click comparisons for full size.</p>"
        + "".join(sections)
    )
    print(f"{len(rows)} support comparisons; outside-support bits preserved")


if __name__ == "__main__":
    main()
