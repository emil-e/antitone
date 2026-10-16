#!/usr/bin/env python3
"""Colour-manage cloud Lightroom JPEGs and compare them with JPEG-matched Antitone repairs."""

import argparse
import hashlib
import html
import io
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image, ImageCms, ImageDraw, JpegImagePlugin


def tiff_u8(path):
    pixels = cv2.imread(str(path), cv2.IMREAD_UNCHANGED)
    if pixels is None or pixels.dtype != np.uint16 or pixels.ndim != 3 or pixels.shape[2] != 3:
        raise ValueError(f"Not an RGB16 TIFF: {path}")
    return Image.fromarray(((pixels[:, :, ::-1].astype(np.uint32) + 128) // 257).astype(np.uint8))


def to_srgb(image, profile, srgb):
    return ImageCms.profileToProfile(
        image.convert("RGB"), profile, srgb, renderingIntent=1, outputMode="RGB"
    )


def jpeg_match(image, cloud, output, srgb, p3, p3_bytes):
    converted = ImageCms.profileToProfile(image, srgb, p3, renderingIntent=1, outputMode="RGB")
    converted.save(
        output,
        format="JPEG",
        qtables=list(cloud.quantization.values()),
        subsampling=JpegImagePlugin.get_sampling(cloud),
        icc_profile=p3_bytes,
    )
    encoded = Image.open(output)
    return to_srgb(encoded, ImageCms.ImageCmsProfile(io.BytesIO(encoded.info["icc_profile"])), srgb)


def highpass(image):
    return image - cv2.GaussianBlur(image, (0, 0), 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--outputs", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=Path("assets/profiles"))
    args = parser.parse_args()
    outputs = args.outputs or args.package / "outputs"
    manifest = json.loads((args.package / "manifest.json").read_text())
    args.output.mkdir(parents=True, exist_ok=False)
    controls = args.output / "jpeg-controls"
    crops = args.output / "crops"
    controls.mkdir()
    crops.mkdir()
    srgb_path = args.profiles / "antitone-srgb-v1.icc"
    p3_path = args.profiles / "antitone-display-p3-v1.icc"
    srgb_bytes, p3_bytes = srgb_path.read_bytes(), p3_path.read_bytes()
    srgb, p3 = ImageCms.getOpenProfile(str(srgb_path)), ImageCms.getOpenProfile(str(p3_path))
    comparisons, mapping = [], {}
    labelled_rows = []
    panel_size = 512
    labelled = Image.new(
        "RGB", (panel_size * 3, (panel_size + 32) * len(manifest["cases"])), "#222"
    )
    blind = Image.new("RGB", (panel_size * 3, (panel_size + 32) * len(manifest["cases"])), "#222")
    labelled_draw, blind_draw = ImageDraw.Draw(labelled), ImageDraw.Draw(blind)
    for row, case in enumerate(manifest["cases"]):
        name = case["case"]
        cloud_path = outputs / (name + ".jpg")
        if not cloud_path.exists():
            raise ValueError(f"Missing Lightroom output: {cloud_path}")
        cloud = Image.open(cloud_path)
        if (
            cloud.size != tuple(case["dimensions"])
            or "icc_profile" not in cloud.info
            or not cloud.quantization
        ):
            raise ValueError(
                f"Lightroom output lacks expected dimensions/profile/JPEG tables: {cloud_path}"
            )
        cloud_profile = ImageCms.ImageCmsProfile(io.BytesIO(cloud.info["icc_profile"]))
        profile_name = ImageCms.getProfileName(cloud_profile).strip()
        lightroom = to_srgb(cloud, cloud_profile, srgb)
        source = tiff_u8(args.package / case["input_tiff"])
        antitone = tiff_u8(args.package / case["antitone_reference_tiff"])
        source_matched = jpeg_match(
            source, cloud, controls / (name + "-input.jpg"), srgb, p3, p3_bytes
        )
        antitone_matched = jpeg_match(
            antitone, cloud, controls / (name + "-antitone.jpg"), srgb, p3, p3_bytes
        )
        mask = np.asarray(Image.open(args.package / case["mask"]).convert("L")) > 0
        outside = cv2.dilate(mask.astype(np.uint8), np.ones((15, 15), np.uint8)) == 0
        source_values = np.asarray(source).astype(np.float32)
        lightroom_values = np.asarray(lightroom).astype(np.float32)
        control_values = np.asarray(source_matched).astype(np.float32)
        base = max(float(np.sqrt(np.mean(highpass(source_values)[outside] ** 2))), 1e-12)
        measurement = {
            "case": name,
            "cloud_profile": profile_name,
            "cloud_dimensions": list(cloud.size),
            "lightroom_outside_support_mae_codes": float(
                np.mean(np.abs(lightroom_values[outside] - source_values[outside]))
            ),
            "matched_jpeg_outside_support_mae_codes": float(
                np.mean(np.abs(control_values[outside] - source_values[outside]))
            ),
            "lightroom_outside_highpass_ratio": float(
                np.sqrt(np.mean(highpass(lightroom_values)[outside] ** 2)) / base
            ),
            "matched_jpeg_outside_highpass_ratio": float(
                np.sqrt(np.mean(highpass(control_values)[outside] ** 2)) / base
            ),
        }
        comparisons.append(measurement)
        x0, y0, x1, y1 = case["zoom"]
        images = {
            "input": source_matched.crop((x0, y0, x1, y1)).resize(
                (panel_size, panel_size), Image.Resampling.NEAREST
            ),
            "antitone": antitone_matched.crop((x0, y0, x1, y1)).resize(
                (panel_size, panel_size), Image.Resampling.NEAREST
            ),
            "lightroom": lightroom.crop((x0, y0, x1, y1)).resize(
                (panel_size, panel_size), Image.Resampling.NEAREST
            ),
        }
        for key, image in images.items():
            image.save(crops / (name + "-" + key + ".png"), icc_profile=srgb_bytes)
        order = ["antitone", "lightroom"]
        if hashlib.sha256(("antitone-lightroom-v1:" + name).encode()).digest()[0] & 1:
            order.reverse()
        mapping[name] = {"A": order[0], "B": order[1]}
        for column, key in enumerate(("input", "antitone", "lightroom")):
            labelled.paste(images[key], (column * panel_size, row * (panel_size + 32) + 32))
            labelled_draw.text(
                (column * panel_size + 5, row * (panel_size + 32) + 7),
                name + " " + key,
                fill="white",
            )
        for column, key in enumerate(("input", order[0], order[1])):
            blind.paste(images[key], (column * panel_size, row * (panel_size + 32) + 32))
            label = "input" if column == 0 else ("A" if column == 1 else "B")
            blind_draw.text(
                (column * panel_size + 5, row * (panel_size + 32) + 7),
                name + " " + label,
                fill="white",
            )
        labelled_rows.append(
            f"<h2>{html.escape(name)}</h2><img src='crops/{html.escape(name, quote=True)}-input.png'>"
            f"<img src='crops/{html.escape(name, quote=True)}-antitone.png'>"
            f"<img src='crops/{html.escape(name, quote=True)}-lightroom.png'>"
        )
    labelled.save(args.output / "labelled.png", icc_profile=srgb_bytes)
    blind.save(args.output / "blind.png", icc_profile=srgb_bytes)
    (args.output / "measurements.json").write_text(json.dumps(comparisons, indent=2))
    (args.output / "mapping.json").write_text(json.dumps(mapping, indent=2))
    (args.output / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Lightroom comparison</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:32%;image-rendering:pixelated}code{background:#333;padding:2px 4px}</style>"
        "<h1>Antitone and Lightroom Cloud</h1><p>Columns below are input, Antitone, Lightroom. Lightroom's embedded "
        "Display P3 profile is converted to Antitone sRGB. Input and Antitone are round-tripped through JPEG using "
        "that case's Lightroom quantization tables and chroma sampling before comparison. This reduces encoding "
        "bias but cannot recover 16-bit data or make different JPEG encoders identical.</p><p>There is no clean "
        "recapture. Outside-support metrics diagnose global JPEG/colour differences only; they do not score repair. "
        "Correction effort and whether Lightroom's first source succeeded remain unknown unless separately recorded.</p>"
        + "".join(labelled_rows)
    )
    print(f"{len(comparisons)} colour-managed, JPEG-matched comparisons")


if __name__ == "__main__":
    main()
