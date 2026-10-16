#!/usr/bin/env python3
"""Fixed-mask, fixed-donor camera-before-inversion versus AP1-after-pipeline study."""

import argparse
import html
import json
from pathlib import Path
import subprocess
import time

import numpy as np
from PIL import Image, ImageDraw

import experiment as e


def develop(probe, crop, camera, mask, output, name):
    camera_path = output / (name + ".camera-f32")
    mask_path = output / (name + ".mask-u8")
    working_path = output / (name + ".ap1-f32")
    camera.astype("<f4").tofile(camera_path)
    mask.astype(np.uint8).tofile(mask_path)
    process = subprocess.run(
        [str(probe), "develop", str(crop), str(camera_path), str(mask_path), str(working_path)],
        check=True,
        capture_output=True,
        text=True,
    )
    working = np.fromfile(working_path, dtype="<f4").reshape(camera.shape)
    statistics = json.loads(process.stdout)
    for path in (camera_path, mask_path, working_path):
        path.unlink()
    return working, statistics


def density_heal(camera, mask, donor_offset=None, method="donor-heal"):
    known = camera[~mask]
    if np.any(known <= 0):
        raise ValueError("Log-camera healing requires strictly positive known transmission")
    density = np.zeros_like(camera)
    density[~mask] = -np.log(known)
    filled, donors = e.repair(density, mask, method, donor_offset=donor_offset)
    result = camera.copy()
    result[mask] = np.exp(-filled[mask])
    if not np.isfinite(result).all() or not e.outside_bit_exact(result, camera, mask):
        raise AssertionError("Log-camera healing overflowed or changed known pixels")
    return result, donors


def donor_diagram(image, mask, donors, output):
    panel = e.overlay(np.asarray(image), mask)
    draw = ImageDraw.Draw(panel)
    for donor in donors:
        x, y, w, h = donor["target"]
        sx, sy = donor["source"]
        draw.rectangle((x, y, x + w - 1, y + h - 1), outline="#00ffff", width=2)
        draw.rectangle((sx, sy, sx + w - 1, sy + h - 1), outline="#ffff00", width=2)
        draw.line((x + w // 2, y + h // 2, sx + w // 2, sy + h // 2), fill="#ffff00")
    panel.save(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crops", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--exposure", type=float, default=2)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in json.loads(args.cases.read_text()):
        crop = args.crops / case["crop"]
        info, camera, working = e.load_crop(crop)
        mask = (
            np.asarray(Image.open(case["mask_file"]).convert("L")) > 0
            if "mask_file" in case
            else e.mask_from_shapes(working.shape, case["shapes"])
        )
        if mask.shape != working.shape[:2] or not mask.any() or mask.all():
            raise ValueError("Invalid study mask")
        out = args.output / case["name"]
        out.mkdir()
        Image.fromarray(mask.astype(np.uint8) * 255).save(out / "mask.png")
        (out / "case.json").write_text(
            json.dumps({"case": case, "source": info, "inspection_ev": args.exposure}, indent=2)
        )
        original = e.render(args.probe, working, out, "original", args.exposure)
        normalized_baseline, floor = develop(
            args.probe, crop, camera, mask, out, "normalization-baseline"
        )
        baseline_error = np.abs(normalized_baseline[mask] - working[mask])
        baseline = {
            "normalization_max_ap1_error": float(baseline_error.max()),
            "normalization_floor": floor,
        }
        assert e.outside_bit_exact(normalized_baseline, working, mask)
        variants = [(method, method, None) for method in e.REPAIRS]
        if "donor_offset" in case:
            variants += [
                ("fixed-clone", "donor-clone", case["donor_offset"]),
                ("fixed-heal", "donor-heal", case["donor_offset"]),
            ]
        for stage in ("camera", "ap1"):
            panels = [
                ("original", original.crop(case["zoom"])),
                ("fixed mask", e.overlay(np.asarray(original), mask).crop(case["zoom"])),
            ]
            stage_variants = variants.copy()
            if stage == "camera":
                stage_variants.append(("density-heal", "donor-heal", None))
                if "donor_offset" in case:
                    stage_variants.append(
                        ("fixed-density-heal", "donor-heal", case["donor_offset"])
                    )
            for name, method, offset in stage_variants:
                start = time.perf_counter()
                if name in ("density-heal", "fixed-density-heal"):
                    repaired, donors = density_heal(camera, mask, donor_offset=offset)
                else:
                    repaired, donors = e.repair(
                        camera if stage == "camera" else working, mask, method, donor_offset=offset
                    )
                repair_seconds = time.perf_counter() - start
                if stage == "camera":
                    result, floor = develop(
                        args.probe, crop, repaired, mask, out, stage + "-" + name
                    )
                    np.save(out / (stage + "-" + name + "-camera-values.npy"), repaired[mask])
                else:
                    result, floor = repaired, None
                if not np.isfinite(result).all() or not e.outside_bit_exact(result, working, mask):
                    raise AssertionError(
                        "Stage comparison changed known pixels or produced nonfinite AP1"
                    )
                np.save(out / (stage + "-" + name + "-masked-values.npy"), result[mask])
                image = e.render(args.probe, result, out, stage + "-" + name, args.exposure)
                panels.append((stage + " " + name, image.crop(case["zoom"])))
                if donors:
                    donor_diagram(
                        original, mask, donors, out / (stage + "-" + name + "-donors.png")
                    )
                row = {
                    "case": case["name"],
                    "kind": case["kind"],
                    "stage": stage,
                    "method": name,
                    "donor_offset": offset,
                    "donors": donors,
                    "mask_pixels": int(mask.sum()),
                    "repair_domain": "log-camera" if "density" in name else stage,
                    "repair_seconds": repair_seconds,
                    "outside_bit_exact": True,
                    "floor": floor,
                    **baseline,
                }
                if case["kind"] == "held-out":
                    row.update(e.measure_repair(working, result, mask))
                rows.append(row)
            filename = stage + "-detail.png"
            e.make_sheet(panels, out / filename, 256)
            sections.append(
                f"<h2>{html.escape(case['name'])} — {html.escape(case['kind'])} — {stage}</h2>"
                f"<p>{html.escape(case.get('notes', ''))}</p>"
                f"<a href='{html.escape(case['name'], quote=True)}/{filename}'>"
                f"<img src='{html.escape(case['name'], quote=True)}/{filename}'></a>"
            )
    (args.output / "measurements.json").write_text(json.dumps(rows, indent=2))
    (args.output / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Repair stage and donor study</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:100%}a{color:#8cf}</style>"
        "<h1>Repair stage and donor selection</h1><p>Native pixels, identical masks. Camera repair precedes the production "
        "scalar inversion/matrix/grade; AP1 repair follows that pipeline. Only selected camera repairs are developed; "
        "known AP1 bytes are retained unchanged. Fixed donors use the same integer translation in both stages, "
        "chosen from masked-context inspection. Automatic donors are selected separately in each representation. "
        "Density-heal uses the identical solver on negative-log native RGB, then exponentiates only repaired pixels: "
        "a multiplicative transmission correction, not a replacement inversion or new floor/clamp.</p>"
        "<p>Visual labels are not clean recaptures. Held-out geometry provides reconstruction truth, not real dust. "
        "Ambiguous marks remain explicitly unverified. Diagnostic calibration and inspection exposure are not "
        "photographic acceptance. Normalization-baseline errors and production floor flags are recorded. "
        "Timings exclude development, rendering and I/O. Click comparisons for full-size detail.</p>"
        + "".join(sections)
    )
    print(f"{len(rows)} stage/donor comparisons; known AP1 bit patterns preserved")


if __name__ == "__main__":
    main()
