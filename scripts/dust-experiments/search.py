#!/usr/bin/env python3
"""Whole-frame native-resolution candidate discovery, NOT automatic dust labelling."""

import argparse
import json
from pathlib import Path
import time

import cv2
import numpy as np
from PIL import Image, ImageDraw

from experiment import analysis_gray, noise_map


def candidates(camera, tile=768, halo=48, polarity="both"):
    if polarity not in ("both", "positive"):
        raise ValueError("Unknown search polarity")
    h, w = camera.shape[:2]
    found = []
    for y in range(0, h, tile):
        for x in range(0, w, tile):
            x0, y0 = max(0, x - halo), max(0, y - halo)
            x1, y1 = min(w, x + tile + halo), min(h, y + tile + halo)
            rgb = camera[y0:y1, x0:x1].astype(np.float32) / 65535
            gray = analysis_gray(rgb)
            noise = noise_map(gray)
            responses = [gray - cv2.GaussianBlur(gray, (0, 0), s) for s in (2, 5, 12)]
            response = np.maximum.reduce(
                responses if polarity == "positive" else [np.abs(r) for r in responses]
            )
            score = response / noise
            count, labels, stats, centers = cv2.connectedComponentsWithStats(
                (score >= 6).astype(np.uint8), connectivity=8
            )
            local = []
            for i in range(1, count):
                bx, by, bw, bh, area = map(int, stats[i])
                cx, cy = centers[i] + (x0, y0)
                if not (x <= cx < min(x + tile, w) and y <= cy < min(y + tile, h)):
                    continue
                if not (64 <= cx < w - 64 and 64 <= cy < h - 64):
                    continue
                if not (6 <= area <= 1800 and 4 <= max(bw, bh) <= 192):
                    continue
                patch_score = score[by : by + bh, bx : bx + bw]
                component = labels[by : by + bh, bx : bx + bw] == i
                peak = float(patch_score[component].max())
                compactness = area / (bw * bh)
                rank = peak * np.sqrt(compactness) / (1 + max(bw, bh) / 32)
                local.append(
                    {
                        "x": int(round(cx)),
                        "y": int(round(cy)),
                        "bbox": [bx + x0, by + y0, bw, bh],
                        "area": area,
                        "peak": peak,
                        "rank": float(rank),
                        "tile": [x, y],
                    }
                )
            # Retain spatial diversity, not just repeated responses from one textured object.
            local.sort(key=lambda c: c["rank"], reverse=True)
            kept = []
            for item in local:
                if all(
                    (item["x"] - other["x"]) ** 2 + (item["y"] - other["y"]) ** 2 > 48**2
                    for other in kept
                ):
                    kept.append(item)
                if len(kept) == 5:
                    break
            found.extend(kept)
    return sorted(found, key=lambda c: c["rank"], reverse=True)


def density_patch(camera, x, y, radius):
    h, w = camera.shape[:2]
    patch = camera[max(0, y - radius) : min(h, y + radius), max(0, x - radius) : min(w, x + radius)]
    gray = analysis_gray(patch.astype(np.float32) / 65535)
    low, high = np.percentile(gray, (1, 99))
    # Explicit diagnostic contrast mapping ONLY. Never feeds processing or repair.
    codes = np.clip((gray - low) / max(high - low, 1e-6), 0, 1)
    return Image.fromarray(np.round(codes * 255).astype(np.uint8)).convert("RGB")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frame", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--polarity", choices=("both", "positive"), default="both")
    args = parser.parse_args()
    start = time.perf_counter()
    info = json.loads((args.frame / "frame.json").read_text())
    camera = np.memmap(
        args.frame / "camera.rgb16", dtype="<u2", mode="r", shape=(info["height"], info["width"], 3)
    )
    args.output.mkdir(parents=True, exist_ok=False)
    found = candidates(camera, polarity=args.polarity)
    for i, item in enumerate(found):
        item["id"] = i
        item["source_xy"] = [item["x"] + info["region"][0], item["y"] + info["region"][1]]
    (args.output / "candidates.json").write_text(
        json.dumps(
            {
                "frame": info,
                "candidates": found,
                "polarity": args.polarity,
                "status": "unlabelled search proposals; contrast-normalized density images are diagnostic only",
                "seconds": time.perf_counter() - start,
            },
            indent=2,
        )
    )
    for page in range((min(args.limit, len(found)) + 24) // 25):
        subset = found[page * 25 : min((page + 1) * 25, args.limit)]
        sheet = Image.new("RGB", (1280, 360 * ((len(subset) + 4) // 5)), "#222222")
        draw = ImageDraw.Draw(sheet)
        for j, item in enumerate(subset):
            ox, oy = (j % 5) * 256, (j // 5) * 360
            draw.text(
                (ox + 3, oy + 3),
                f"#{item['id']} {item['source_xy']} s={item['peak']:.1f}",
                fill="white",
            )
            context = density_patch(camera, item["x"], item["y"], 96)
            sheet.paste(context, (ox, oy + 24))
            zoom = density_patch(camera, item["x"], item["y"], 16).resize(
                (128, 128), Image.Resampling.NEAREST
            )
            sheet.paste(zoom, (ox, oy + 224))
        sheet.save(args.output / f"atlas-{page}.png")
    print(
        f"{Path(info['source']).name}: {len(found)} unlabelled proposals, {time.perf_counter() - start:.1f}s"
    )


if __name__ == "__main__":
    main()
