#!/usr/bin/env python3
"""Run the frozen dust-repair candidate once on preselected, previously unseen cases."""

import argparse
import hashlib
import html
import json
from pathlib import Path
import time

import numpy as np
from PIL import Image

import experiment as e
from mask_study import score_support
import stage_study as stages


CANDIDATE = "ap1-boundary-ranked-score10-grow4-v1"
THRESHOLD = 10
GROWTH = 4


def verify_freeze(case_bytes, freeze):
    digest = hashlib.sha256(case_bytes).hexdigest()
    if freeze.get("case_manifest_sha256") != digest or freeze.get("candidate") != CANDIDATE:
        raise ValueError("Case manifest or candidate does not match the pre-repair freeze record")
    return json.loads(case_bytes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crops", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--freeze", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--exposure", type=float, default=2)
    args = parser.parse_args()
    case_bytes = args.cases.read_bytes()
    freeze = json.loads(args.freeze.read_text())
    cases = verify_freeze(case_bytes, freeze)
    names = [case["name"] for case in cases]
    if len(names) != len(set(names)):
        raise ValueError("Case names must be unique")
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in cases:
        crop = args.crops / case["crop"]
        info, camera, working = e.load_crop(crop)
        annotation = e.mask_from_shapes(working.shape, case["shapes"])
        if not annotation.any() or annotation.all():
            raise ValueError(f"Invalid annotation for {case['name']}")
        score = e.positive_gaussian_score(camera)
        out = args.output / case["name"]
        out.mkdir()
        original = e.render(args.probe, working, out, "original", args.exposure)
        panels = [
            ("original", original.crop(case["zoom"])),
            ("frozen annotation", e.overlay(np.asarray(original), annotation).crop(case["zoom"])),
        ]
        row = {
            "case": case["name"],
            "kind": case["kind"],
            "candidate": CANDIDATE,
            "annotation_pixels": int(annotation.sum()),
            "score_max_in_annotation": float(score[annotation].max()),
            "source_floor_pixels": info["floor_pixels"],
        }
        try:
            core, support = score_support(annotation, score, THRESHOLD, GROWTH)
        except ValueError as error:
            row.update(
                {
                    "status": "rejected-no-score-10-core",
                    "reason": str(error),
                    "core_pixels": 0,
                    "support_pixels": 0,
                    "donors": [],
                }
            )
            panels.append(("automatic support rejected", original.crop(case["zoom"])))
        else:
            Image.fromarray(core.astype(np.uint8) * 255).save(out / "core.png")
            Image.fromarray(support.astype(np.uint8) * 255).save(out / "support.png")
            start = time.perf_counter()
            result, donors = e.repair(working, support, "donor-heal-ranked")
            elapsed = time.perf_counter() - start
            if not np.array_equal(
                result[~support].view(np.uint32), working[~support].view(np.uint32)
            ):
                raise ValueError("Repair changed known AP1 bits")
            repaired = e.render(args.probe, result, out, "repaired", args.exposure)
            np.save(out / "repaired-masked-values.npy", result[support])
            stages.donor_diagram(original, support, donors, out / "donors.png")
            panels.extend(
                (
                    ("score-10 core", e.overlay(np.asarray(original), core).crop(case["zoom"])),
                    (
                        "score-10 + grow-4 support",
                        e.overlay(np.asarray(original), support).crop(case["zoom"]),
                    ),
                    ("frozen repair", repaired.crop(case["zoom"])),
                )
            )
            row.update(
                {
                    "status": "repaired",
                    "core_pixels": int(core.sum()),
                    "support_pixels": int(support.sum()),
                    "support_fraction_of_annotation": float(support.sum() / annotation.sum()),
                    "donors": donors,
                    "repair_seconds": elapsed,
                    "outside_support_bit_exact": True,
                }
            )
        e.make_sheet(panels, out / "detail.png", 384)
        (out / "case.json").write_text(
            json.dumps(
                {"case": case, "source": info, "freeze": freeze, "inspection_ev": args.exposure},
                indent=2,
            )
        )
        rows.append(row)
        escaped = html.escape(case["name"], quote=True)
        sections.append(
            f"<h2>{escaped} — {html.escape(row['status'])}</h2>"
            f"<p>{html.escape(case.get('notes', ''))}</p>"
            f"<a href='{escaped}/detail.png'><img src='{escaped}/detail.png'></a>"
        )
        print(case["name"], row["status"], flush=True)
    repaired_count = sum(row["status"] == "repaired" for row in rows)
    (args.output / "measurements.json").write_text(
        json.dumps({"candidate": CANDIDATE, "freeze": freeze, "cases": rows}, indent=2)
    )
    (args.output / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Independent dust repair validation</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:100%}a{color:#8cf}</style>"
        "<h1>Frozen candidate on independently selected scans</h1><p>Scan names and visual annotations were frozen "
        "before any repair was generated. The only candidate is AP1 boundary-ranked donor healing with a positive "
        "Gaussian score-10 core and four-pixel growth clipped to the annotation. A missing core rejects rather than "
        "falling back or changing the threshold. No parameter or mask is adjusted after viewing these results.</p>"
        "<p>These are proposal-assisted visual labels, not exhaustive detector labels, and there are no clean "
        "recaptures. Inspection can identify residual defects, visible seams, wrong structure and texture mismatch; "
        "it cannot reveal the true covered pixels. Diagnostic calibration and +2 EV rendering are not accepted "
        "roll calibration. Every accepted repair preserves signed/HDR AP1 f32 bits outside its displayed support.</p>"
        + "".join(sections)
    )
    print(f"{repaired_count}/{len(rows)} cases repaired by the frozen candidate")


if __name__ == "__main__":
    main()
