#!/usr/bin/env python3
"""Bounded, offline dust detection/repair comparisons. No editor integration."""
import argparse
import html
import json
import os
from pathlib import Path
import subprocess
import time

import cv2
import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage, sparse
from scipy.sparse.linalg import splu

DETECTORS = ("gaussian", "median", "morphology")
REPAIRS = ("harmonic", "telea", "navier-stokes", "donor-clone", "donor-heal")
DONOR_REFINEMENTS = ("donor-heal-local", "donor-heal-structure", "donor-heal-refined",
                     "donor-heal-boundary", "donor-heal-ranked")


def load_crop(directory):
    info = json.loads((directory / "source.json").read_text())
    if info["stride"] != 1:
        raise ValueError("Comparisons require native-resolution crops, not survey proxies")
    shape = (info["height"], info["width"], 3)
    camera = np.fromfile(directory / "camera.rgb16", dtype="<u2").reshape(shape).astype(np.float32) / 65535
    working = np.fromfile(directory / "working.ap1-f32", dtype="<f4").reshape(shape)
    if not np.isfinite(working).all():
        raise ValueError("Nonfinite working pixels")
    return info, camera, working


def analysis_gray(camera):
    # This is a detection representation, not a replacement inversion/development.
    return -np.log(np.maximum(camera, 1 / 65535)).mean(axis=2).astype(np.float32)


def noise_map(gray, tile=32):
    residual = np.abs(gray - cv2.GaussianBlur(gray, (0, 0), 1))
    h, w = gray.shape
    grid = np.empty(((h + tile - 1) // tile, (w + tile - 1) // tile), np.float32)
    for y in range(grid.shape[0]):
        for x in range(grid.shape[1]):
            grid[y, x] = 1.4826 * np.median(residual[y * tile:(y + 1) * tile, x * tile:(x + 1) * tile])
    floor = max(float(np.median(grid)) * 0.3, 1e-6)
    return np.maximum(cv2.resize(grid, (w, h), interpolation=cv2.INTER_LINEAR), floor)


def positive_gaussian_score(camera):
    gray = analysis_gray(camera)
    residual = np.maximum.reduce([gray - cv2.GaussianBlur(gray, (0, 0), s) for s in (2, 5, 12)])
    return residual / noise_map(gray)


def detection_scores(camera):
    gray = analysis_gray(camera)
    noise = noise_map(gray)
    scores = {}
    timings = {}
    for method in DETECTORS:
        start = time.perf_counter()
        residuals = []
        if method == "gaussian":
            for sigma in (2, 5, 12):
                residuals.append(np.abs(gray - cv2.GaussianBlur(gray, (0, 0), sigma)))
        elif method == "median":
            for size in (7, 13):
                residuals.append(np.abs(gray - ndimage.median_filter(gray, size=size, mode="reflect")))
        else:
            for size in (11, 25):
                kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (size, size))
                for operation in (cv2.MORPH_TOPHAT, cv2.MORPH_BLACKHAT):
                    residuals.append(cv2.morphologyEx(gray, operation, kernel))
        scores[method] = np.maximum.reduce(residuals) / noise
        timings[method] = time.perf_counter() - start
    return scores, timings


def candidate_mask(score, threshold):
    selected = (score >= threshold).astype(np.uint8)
    selected[:16] = selected[-16:] = 0
    selected[:, :16] = selected[:, -16:] = 0
    count, labels, stats, _ = cv2.connectedComponentsWithStats(selected, connectivity=8)
    accepted = np.zeros(count, bool)
    for i in range(1, count):
        _, _, width, height, area = stats[i]
        accepted[i] = 3 <= area <= 2048 and max(width, height) <= 256
    return accepted[labels]


def overlay(original, mask):
    output = original.copy()
    output[mask] = (255, 0, 255)
    outline = cv2.dilate(mask.astype(np.uint8), np.ones((3, 3), np.uint8)) > 0
    output[outline & ~mask] = (0, 255, 255)
    return Image.fromarray(output)


def mask_from_shapes(shape, shapes):
    mask = np.zeros(shape[:2], np.uint8)
    for item in shapes:
        if item["kind"] == "circle":
            cv2.circle(mask, tuple(item["center"]), item["radius"], 1, -1)
        elif item["kind"] == "ellipse":
            cv2.ellipse(mask, tuple(item["center"]), tuple(item["axes"]), item.get("angle", 0), 0, 360, 1, -1)
        elif item["kind"] == "line":
            cv2.line(mask, tuple(item["start"]), tuple(item["end"]), 1, item["width"])
        else:
            raise ValueError("Unknown shape")
    return mask.astype(bool)


def harmonic_fill(image, mask):
    """4-neighbour Dirichlet Laplace solve; original pixels inside mask are unused."""
    output = image.copy()
    ys, xs = np.nonzero(mask)
    if not len(ys):
        return output
    if mask.all():
        raise ValueError("No valid boundary")
    indices = np.full(mask.shape, -1, np.int32)
    indices[ys, xs] = np.arange(len(ys))
    rows, cols, values = [], [], []
    rhs = np.zeros((len(ys), 3), np.float64)
    for i, (y, x) in enumerate(zip(ys, xs)):
        degree = 0
        for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
            ny, nx = y + dy, x + dx
            if not (0 <= ny < mask.shape[0] and 0 <= nx < mask.shape[1]):
                continue
            degree += 1
            if mask[ny, nx]:
                rows.append(i); cols.append(int(indices[ny, nx])); values.append(-1)
            else:
                rhs[i] += image[ny, nx]
        rows.append(i); cols.append(i); values.append(degree)
    matrix = sparse.csc_matrix((values, (rows, cols)), shape=(len(ys), len(ys)), dtype=np.float64)
    output[ys, xs] = splu(matrix).solve(rhs).astype(np.float32)
    return output


def biharmonic_fill(image, mask, iterations=None):
    """Fixed-exterior bilaplacian: exact solve, or bounded refinement of a harmonic fill."""
    output = image.copy()
    ys, xs = np.nonzero(mask)
    if not len(ys):
        return output
    if ys.min() < 2 or xs.min() < 2 or ys.max() >= mask.shape[0] - 2 or xs.max() >= mask.shape[1] - 2:
        raise ValueError("Biharmonic correction requires a two-pixel exterior")
    kernel = ((0, 0, 20), (-1, 0, -8), (1, 0, -8), (0, -1, -8), (0, 1, -8),
              (-2, 0, 1), (2, 0, 1), (0, -2, 1), (0, 2, 1),
              (-1, -1, 2), (-1, 1, 2), (1, -1, 2), (1, 1, 2))
    if iterations is not None:
        correction = harmonic_fill(image, mask).astype(np.float64)
        stencil = np.zeros((5, 5), np.float64)
        for dy, dx, weight in kernel:
            stencil[dy + 2, dx + 2] = weight
        # The squared four-neighbour Laplacian has spectral radius at most 64.
        # A few stable steps adjust the boundary without converging the whole interior.
        for _ in range(iterations):
            residual = cv2.filter2D(correction, cv2.CV_64F, stencil)
            correction[mask] -= residual[mask] / 64
        output[mask] = correction[mask].astype(np.float32)
        return output
    indices = np.full(mask.shape, -1, np.int32)
    indices[ys, xs] = np.arange(len(ys))
    rows, cols, values = [], [], []
    rhs = np.zeros((len(ys), 3), np.float64)
    for i, (y, x) in enumerate(zip(ys, xs)):
        for dy, dx, weight in kernel:
            ny, nx = y + dy, x + dx
            if mask[ny, nx]:
                rows.append(i); cols.append(int(indices[ny, nx])); values.append(weight)
            else:
                rhs[i] -= weight * image[ny, nx].astype(np.float64)
    matrix = sparse.csc_matrix((values, (rows, cols)), shape=(len(ys), len(ys)), dtype=np.float64)
    output[ys, xs] = splu(matrix).solve(rhs).astype(np.float32)
    return output


def opencv_fill(image, mask, method):
    # OpenCV float Telea is not scale-invariant (even constant [0,1] fields drift).
    # Condition floats to a code-sized range, WITHOUT clipping or RGB8 conversion.
    # Bounds use known pixels only, never the reference values hidden by the mask.
    known = image[~mask]
    low, high = float(known.min()), float(known.max())
    span = high - low
    if span == 0:
        span = max(abs(high), 1 / 65535)
    gain = 255 / span
    encoded = ((image - low) * gain).astype(np.float32)
    encoded[mask] = 0
    result = image.copy()
    flag = cv2.INPAINT_TELEA if method == "telea" else cv2.INPAINT_NS
    for channel in range(3):
        filled = cv2.inpaint(encoded[:, :, channel], mask.astype(np.uint8), 3, flag)
        result[:, :, channel][mask] = filled[mask] / gain + low
    return result


def donor_fill(image, mask, heal, search_radius=128, donor_offset=None):
    output = image.copy()
    count, labels, stats, _ = cv2.connectedComponentsWithStats(mask.astype(np.uint8), connectivity=8)
    donors = []
    for i in range(1, count):
        x, y, w, h, _ = map(int, stats[i])
        margin = 10
        x0, y0 = max(0, x - margin), max(0, y - margin)
        x1, y1 = min(image.shape[1], x + w + margin), min(image.shape[0], y + h + margin)
        target = image[y0:y1, x0:x1]
        local = labels[y0:y1, x0:x1] == i
        known = ~mask[y0:y1, x0:x1]
        if not known.any():
            raise ValueError("No donor matching context")
        if donor_offset is None:
            sy0, sx0 = max(0, y0 - search_radius), max(0, x0 - search_radius)
            sy1, sx1 = min(image.shape[0], y1 + search_radius), min(image.shape[1], x1 + search_radius)
            search = image[sy0:sy1, sx0:sx1].copy()
            search_unknown = mask[sy0:sy1, sx0:sx1]
            search[search_unknown] = 0
            template = target.copy()
            template[~known] = 0
            score = cv2.matchTemplate(search, template, cv2.TM_SQDIFF, mask=known.astype(np.uint8))
            contaminated = cv2.matchTemplate(search_unknown.astype(np.float32), np.ones(known.shape, np.float32), cv2.TM_CCORR)
            score[contaminated > 0.5] = np.inf
            if not np.isfinite(score).any():
                raise ValueError("No unmasked donor in search region")
            dy, dx = np.unravel_index(np.argmin(score), score.shape)
            source_x, source_y = sx0 + int(dx), sy0 + int(dy)
            context_mse = float(score[dy, dx] / (known.sum() * 3))
        else:
            if len(donor_offset) != 2 or any(not isinstance(v, int) for v in donor_offset):
                raise ValueError("Donor offset must be two integer pixels")
            dx, dy = donor_offset
            source_x, source_y = x0 + dx, y0 + dy
            if source_x < 0 or source_y < 0 or source_x + target.shape[1] > image.shape[1] or source_y + target.shape[0] > image.shape[0]:
                raise ValueError("Manual donor outside crop")
            if mask[source_y:source_y + target.shape[0], source_x:source_x + target.shape[1]].any():
                raise ValueError("Manual donor intersects unknown pixels")
            donor = image[source_y:source_y + target.shape[0], source_x:source_x + target.shape[1]]
            context_mse = float(np.mean((target[known].astype(np.float64) - donor[known]) ** 2))
        donor = image[source_y:source_y + target.shape[0], source_x:source_x + target.shape[1]]
        if heal:
            # Preserve the donor's gradients, solve a smooth boundary correction.
            difference = target - donor
            difference[local] = 0
            repaired = donor + harmonic_fill(difference, local)
        else:
            repaired = donor
        output[y0:y1, x0:x1][local] = repaired[local]
        donors.append({"target": [x0, y0, x1 - x0, y1 - y0], "source": [source_x, source_y],
                       "context_mse": context_mse})
    return output, donors


def paired_scores(search, source_known, template, target_weights):
    """Weighted SSD over candidate-specific known pairs, not zero-filled missing context."""
    a, weights = source_known.astype(np.float32), target_weights.astype(np.float32)
    count = cv2.matchTemplate(a, weights, cv2.TM_CCORR).astype(np.float64)
    source = search * a[:, :, None]
    target = template * weights[:, :, None]
    total = cv2.matchTemplate(np.sum(source ** 2, axis=2), weights, cv2.TM_CCORR).astype(np.float64)
    total += cv2.matchTemplate(a, np.sum(template ** 2, axis=2) * weights, cv2.TM_CCORR)
    for channel in range(template.shape[2]):
        total -= 2 * cv2.matchTemplate(source[:, :, channel], target[:, :, channel], cv2.TM_CCORR).astype(np.float64)
    # Correlation arithmetic can make a theoretically nonnegative metric slightly negative.
    return np.maximum(total, 0) / (np.maximum(count, 1e-12) * template.shape[2]), count


def nearest_competitive_candidate(ranked, tolerance=0.01):
    """Prefer proximity among candidates within a small multiplicative score tolerance."""
    best = min(item[0] for item in ranked)
    competitive = [item for item in ranked if item[0] <= best * (1 + tolerance) + 1e-12]
    return min(competitive, key=lambda item: (item[3], item[0], item[1], item[2]))


def refined_donor_fill(image, mask, method, search_radius=128, donor_offset=None):
    structure = method != "donor-heal-local"
    ranked_match = method == "donor-heal-ranked"
    boundary = {"donor-heal-refined": "biharmonic", "donor-heal-boundary": "limited-biharmonic",
                "donor-heal-ranked": "limited-biharmonic"}.get(method, "harmonic")
    output, donors = image.copy(), []
    # A single donor/correction jointly handles masks whose two-pixel stencils interact.
    expanded = cv2.dilate(mask.astype(np.uint8), np.ones((3, 3), np.uint8))
    count, groups = cv2.connectedComponents(expanded, connectivity=8)
    if structure:
        smooth = cv2.GaussianBlur(image, (7, 7), 1)
        gradients = np.concatenate((cv2.Sobel(smooth, cv2.CV_32F, 1, 0, ksize=3, scale=1 / 8),
                                    cv2.Sobel(smooth, cv2.CV_32F, 0, 1, ksize=3, scale=1 / 8)), axis=2)
        gradient_known = cv2.dilate(mask.astype(np.uint8), np.ones((9, 9), np.uint8)) == 0
    for group in range(1, count):
        component = mask & (groups == group)
        ys, xs = np.nonzero(component)
        x0, y0 = max(0, int(xs.min()) - 10), max(0, int(ys.min()) - 10)
        x1, y1 = min(image.shape[1], int(xs.max()) + 11), min(image.shape[0], int(ys.max()) + 11)
        target = image[y0:y1, x0:x1]
        local = component[y0:y1, x0:x1]
        known = ~mask[y0:y1, x0:x1]
        # Identical source eligibility for harmonic/biharmonic isolates the correction.
        required = cv2.dilate(local.astype(np.uint8), np.ones((5, 5), np.uint8)) > 0
        if np.any(required & mask[y0:y1, x0:x1] & ~local):
            raise AssertionError("Dependent masks were not grouped")
        if boundary != "harmonic" and (ys.min() < 2 or xs.min() < 2 or ys.max() >= image.shape[0] - 2 or xs.max() >= image.shape[1] - 2):
            raise ValueError("Biharmonic correction requires a two-pixel exterior")
        center = target[known].astype(np.float64).mean(axis=0)
        contrast = float(np.sqrt(np.mean((target[known].astype(np.float64) - center) ** 2)))
        contrast = max(contrast, float(np.abs(center).max()) * 1e-6, 1e-12)
        color = ((image.astype(np.float64) - center) / contrast).astype(np.float32)
        color[mask] = 0
        match_weights = known.astype(np.float32)
        if ranked_match:
            distance = cv2.distanceTransform(known.astype(np.uint8), cv2.DIST_L2, 5)
            match_weights *= 0.15 + 0.85 * np.exp(-np.maximum(distance - 1, 0) / 3)
        features = [(color, ~mask, known, match_weights, 1.0)]
        if structure:
            valid = gradient_known[y0:y1, x0:x1]
            if not valid.any():
                raise ValueError("No known structure context")
            scale = max(float(np.sqrt(np.mean(gradients[y0:y1, x0:x1][valid].astype(np.float64) ** 2))), contrast * 1e-6)
            color_weight, gradient_weight = (0.75, 0.25) if ranked_match else (0.5, 0.5)
            features = [(color, ~mask, known, match_weights, color_weight),
                        ((gradients / scale).astype(np.float32), gradient_known, valid,
                         match_weights * valid, gradient_weight)]
        h, w = local.shape
        sy0, sx0 = max(0, y0 - search_radius), max(0, x0 - search_radius)
        sy1, sx1 = min(image.shape[0], y1 + search_radius), min(image.shape[1], x1 + search_radius)
        if donor_offset is None:
            contaminated = cv2.matchTemplate(mask[sy0:sy1, sx0:sx1].astype(np.float32),
                                             required.astype(np.float32), cv2.TM_CCORR) > 0.5
            score = np.zeros(contaminated.shape, np.float64)
            eligible = ~contaminated
            for feature, valid_source, valid_target, weights, mixture in features:
                metric, pairs = paired_scores(feature[sy0:sy1, sx0:sx1], valid_source[sy0:sy1, sx0:sx1],
                                               feature[y0:y1, x0:x1], weights)
                score += mixture * metric
                eligible &= pairs >= max(1e-12, float(weights.sum()) * 0.5)
            score[~eligible] = np.inf
            if not np.isfinite(score).any():
                raise ValueError("No eligible donor in search region")
            # Re-evaluate the shortlist directly in f64 to avoid correlation cancellation.
            order = np.argsort(score, axis=None, kind="stable")
            limit = min(16, len(order))
            if ranked_match:
                threshold = float(score.flat[order[0]]) * 1.01 + 1e-12
                limit = max(limit, int(np.searchsorted(score.ravel()[order], threshold, side="right")))
            candidates = []
            for index in order[:limit]:
                dy, dx = np.unravel_index(index, score.shape)
                if np.isfinite(score[dy, dx]):
                    candidates.append((sx0 + int(dx), sy0 + int(dy)))
        else:
            if len(donor_offset) != 2 or any(not isinstance(v, int) for v in donor_offset):
                raise ValueError("Donor offset must be two integer pixels")
            dx, dy = donor_offset
            sx, sy = x0 + dx, y0 + dy
            if sx < 0 or sy < 0 or sx + w > image.shape[1] or sy + h > image.shape[0]:
                raise ValueError("Manual donor outside crop")
            if np.any(mask[sy:sy + h, sx:sx + w] & required):
                raise ValueError("Manual donor intersects required source pixels")
            candidates = [(sx, sy)]
        ranked = []
        for sx, sy in candidates:
            exact = 0.0
            for feature, valid_source, valid_target, weights, mixture in features:
                paired = valid_target & valid_source[sy:sy + h, sx:sx + w]
                paired_weight = weights[paired].astype(np.float64)
                if paired_weight.sum() < max(1e-12, float(weights.sum()) * 0.5):
                    raise ValueError("Insufficient paired donor context")
                delta = feature[y0:y1, x0:x1][paired].astype(np.float64) - feature[sy:sy + h, sx:sx + w][paired]
                exact += mixture * float(np.sum(delta ** 2 * paired_weight[:, None]) /
                                         (paired_weight.sum() * delta.shape[1]))
            distance = float(np.hypot(sx - x0, sy - y0))
            ranked.append((exact, sy, sx, distance))
        if ranked_match and donor_offset is None:
            matching_score, source_y, source_x, _ = nearest_competitive_candidate(ranked)
        else:
            matching_score, source_y, source_x, _ = min(ranked)
        donor = image[source_y:source_y + h, source_x:source_x + w]
        paired = known & ~mask[source_y:source_y + h, source_x:source_x + w]
        context_mse = float(np.mean((target[paired].astype(np.float64) - donor[paired]) ** 2))
        difference = np.zeros_like(target)
        exterior = required & ~local
        difference[exterior] = target[exterior] - donor[exterior]
        if boundary == "limited-biharmonic":
            correction = biharmonic_fill(difference, local, iterations=8)
        else:
            solver = biharmonic_fill if boundary == "biharmonic" else harmonic_fill
            correction = solver(difference, local)
        repaired = donor + correction
        output[y0:y1, x0:x1][local] = repaired[local]
        donors.append({"target": [x0, y0, w, h], "source": [source_x, source_y],
                       "context_mse": context_mse, "paired_context_pixels": int(paired.sum()),
                       "matching_score": matching_score,
                       "matcher": "boundary-ranked" if ranked_match else ("structure" if structure else "rgb"),
                       "boundary": boundary, "source_support": "core-and-two-pixel-exterior"})
    return output, donors


def outside_bit_exact(a, b, mask):
    return np.array_equal(a[~mask].view(np.uint32), b[~mask].view(np.uint32))


def repair(image, mask, method, donor_offset=None):
    if image.dtype != np.float32 or not np.isfinite(image).all():
        raise ValueError("Expected finite float32 RGB")
    if mask.shape != image.shape[:2] or mask.all():
        raise ValueError("Invalid repair mask")
    source = image.copy()
    source[mask] = 0  # Prevent accidental access to held-out truth or contaminated pixels.
    if method == "harmonic":
        result, donors = harmonic_fill(source, mask), []
    elif method in ("telea", "navier-stokes"):
        result, donors = opencv_fill(source, mask, method), []
    elif method in ("donor-clone", "donor-heal"):
        result, donors = donor_fill(source, mask, method == "donor-heal", donor_offset=donor_offset)
    elif method in DONOR_REFINEMENTS:
        result, donors = refined_donor_fill(source, mask, method, donor_offset=donor_offset)
    else:
        raise ValueError(method)
    if not np.isfinite(result).all() or not outside_bit_exact(result, image, mask):
        raise AssertionError("Repair changed outside-mask values or produced nonfinite output")
    return result, donors


def measure_repair(truth, result, mask):
    error = result[mask] - truth[mask]
    band = (cv2.dilate(mask.astype(np.uint8), np.ones((7, 7), np.uint8)) > 0) & ~mask
    def highpass(values):
        return values - cv2.GaussianBlur(values, (0, 0), 1)
    a, b = highpass(truth), highpass(result)
    reference_energy = np.sqrt(np.mean(a[mask] ** 2))
    return {"masked_rmse": float(np.sqrt(np.mean(error ** 2))),
            "masked_mae": float(np.mean(np.abs(error))),
            "masked_highpass_ratio": float(np.sqrt(np.mean(b[mask] ** 2)) / max(reference_energy, 1e-12)),
            "boundary_context_highpass_rms": float(np.sqrt(np.mean(a[band] ** 2))),
            "outside_bit_exact": bool(outside_bit_exact(truth, result, mask))}


def render(probe, pixels, directory, name, exposure):
    h, w = pixels.shape[:2]
    path = directory / (name + ".ap1-f32")
    pixels.astype("<f4").tofile(path)
    ppm = directory / (name + ".ppm")
    subprocess.run([str(probe), "render", str(path), str(w), str(h), str(ppm), str(exposure)],
                   check=True, stdout=subprocess.DEVNULL)
    image = Image.open(ppm).convert("RGB")
    image.save(directory / (name + ".png"))
    path.unlink()
    ppm.unlink()
    return image


def make_sheet(panels, output, size=384):
    sheet = Image.new("RGB", (size * len(panels), size + 32), "#222222")
    draw = ImageDraw.Draw(sheet)
    for i, (label, image) in enumerate(panels):
        image = image.copy()
        scale = min(size / image.width, size / image.height)
        image = image.resize((round(image.width * scale), round(image.height * scale)), Image.Resampling.NEAREST)
        sheet.paste(image, (size * i, 32))
        draw.text((size * i + 5, 8), label, fill="white")
    sheet.save(output)


def detect(args):
    args.output.mkdir(parents=True, exist_ok=False)
    report = []
    for directory in sorted(args.crops.iterdir()):
        if not (directory / "source.json").exists():
            continue
        info, camera, _ = load_crop(directory)
        out = args.output / directory.name
        out.mkdir()
        original = Image.open(directory / "inspection-plus2.png").convert("RGB")
        scores, timings = detection_scores(camera)
        panels = [("original", original)]
        for method, score in scores.items():
            np.save(out / (method + "-scores.npy"), score)
            for threshold in (4, 6, 8, 10):
                mask = candidate_mask(score, threshold)
                Image.fromarray(mask.astype(np.uint8) * 255).save(out / f"{method}-{threshold}-mask.png")
                report.append({"crop": directory.name, "source": info["source"], "method": method,
                               "threshold": threshold, "pixels": int(mask.sum()),
                               "components": int(cv2.connectedComponents(mask.astype(np.uint8), connectivity=8)[0] - 1),
                               "score_seconds": timings[method]})
                if threshold == 6:
                    picture = overlay(np.asarray(original), mask)
                    picture.save(out / (method + "-overlay.png"))
                    panels.append((method + " threshold=6", picture))
        make_sheet(panels, out / "comparison.png")
    (args.output / "measurements.json").write_text(json.dumps(report, indent=2))


def repairs(args):
    args.output.mkdir(parents=True, exist_ok=False)
    cases = json.loads(args.cases.read_text())
    report = []
    for case in cases:
        crop = args.crops / case["crop"]
        info, _, working = load_crop(crop)
        if "mask_file" in case:
            mask = np.asarray(Image.open(case["mask_file"]).convert("L")) > 0
        else:
            mask = mask_from_shapes(working.shape, case["shapes"])
        if mask.shape != working.shape[:2] or not mask.any():
            raise ValueError("Case mask is empty or has incorrect dimensions")
        ys, xs = np.nonzero(mask)
        cx, cy = int((xs.min() + xs.max()) // 2), int((ys.min() + ys.max()) // 2)
        if max(int(xs.max() - xs.min()), int(ys.max() - ys.min())) > 256:
            x0, y0, x1, y1 = 0, 0, working.shape[1], working.shape[0]
        else:
            x0, y0 = max(0, cx - 192), max(0, cy - 192)
            x1, y1 = min(working.shape[1], cx + 192), min(working.shape[0], cy + 192)
        working, mask = working[y0:y1, x0:x1].copy(), mask[y0:y1, x0:x1].copy()
        out = args.output / case["name"]
        out.mkdir()
        (out / "case.json").write_text(json.dumps({"case": case, "crop_window": [x0, y0, x1, y1],
                                                   "inspection_ev": args.exposure}, indent=2))
        Image.fromarray(mask.astype(np.uint8) * 255).save(out / "mask.png")
        original = render(args.probe, working, out, "original", args.exposure)
        panels = [("original", original), ("fixed mask", overlay(np.asarray(original), mask))]
        zoom = [case["zoom"][0] - x0, case["zoom"][1] - y0,
                case["zoom"][2] - x0, case["zoom"][3] - y0]
        zoom_panels = [("original", original.crop(zoom)), ("fixed mask", overlay(np.asarray(original), mask).crop(zoom))]
        for method in REPAIRS:
            start = time.perf_counter()
            result, donors = repair(working, mask, method)
            seconds = time.perf_counter() - start
            np.save(out / (method + "-masked-values.npy"), result[mask])
            image = render(args.probe, result, out, method, args.exposure)
            panels.append((method, image)); zoom_panels.append((method, image.crop(zoom)))
            row = {"case": case["name"], "crop": case["crop"], "source": info["source"],
                   "kind": case["kind"], "method": method, "mask_pixels": int(mask.sum()),
                   "repair_window": [x0, y0, x1, y1],
                   "repair_seconds": seconds, "donors": donors}
            if case["kind"] == "held-out":
                row.update(measure_repair(working, result, mask))
            else:
                row["outside_bit_exact"] = bool(outside_bit_exact(working, result, mask))
            report.append(row)
        make_sheet(panels, out / "comparison.png", 256)
        make_sheet(zoom_panels, out / "detail.png", 256)
    (args.output / "measurements.json").write_text(json.dumps(report, indent=2))


def synthetic_detection(args):
    args.output.mkdir(parents=True, exist_ok=False)
    cases = json.loads(args.cases.read_text())
    report = []
    for case in cases:
        if case["kind"] != "held-out":
            continue
        _, camera, _ = load_crop(args.crops / case["crop"])
        support = mask_from_shapes(camera.shape, case["shapes"])
        alpha = cv2.GaussianBlur(support.astype(np.float32), (0, 0), 1)
        truth = alpha >= 0.1
        for opacity in (0.25, 0.75):
            damaged = camera * (1 - opacity * alpha[:, :, None])
            scores, _ = detection_scores(damaged)
            for method, score in scores.items():
                baseline = np.load(args.baseline_scores / case["crop"] / (method + "-scores.npy"))
                operating_points = [("fixed-score", None, t) for t in (4, 6, 8, 10)]
                # Scores have different null distributions; a shared score is not a fair operating point.
                for budget in (0.001, 0.005):
                    threshold = float(np.quantile(baseline[16:-16, 16:-16], 1 - budget))
                    operating_points.append(("baseline-quantile", budget, threshold))
                for mode, budget, threshold in operating_points:
                    mask = candidate_mask(score, threshold)
                    baseline_mask = candidate_mask(baseline, threshold)
                    hit = int((mask & truth).sum())
                    # Background includes unlabelled real defects; NOT a measured false-positive rate.
                    background = int((mask & ~truth).sum())
                    report.append({"case": case["name"], "crop": case["crop"], "opacity": opacity,
                                   "method": method, "mode": mode, "baseline_tail_budget": budget, "threshold": threshold,
                                   "baseline_flagged_pixels": int(baseline_mask.sum()),
                                   "support_coverage": hit / int(truth.sum()),
                                   "baseline_support_coverage": int((baseline_mask & truth).sum()) / int(truth.sum()),
                                   "background_flagged_pixels": background,
                                   "support_pixels": int(truth.sum())})
    (args.output / "measurements.json").write_text(json.dumps(report, indent=2))


def labelled_detection(args):
    """Annotation coverage, not recall: proposals were selected with a Gaussian detector."""
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in json.loads(args.cases.read_text()):
        crop = args.crops / case["crop"]
        _, camera, _ = load_crop(crop)
        support = mask_from_shapes(camera.shape, case["shapes"])
        if not support.any():
            raise ValueError("Empty annotation")
        scores = {method: np.load(args.baseline_scores / case["crop"] / (method + "-scores.npy")) for method in DETECTORS}
        scores["gaussian-positive"] = positive_gaussian_score(camera)
        original = Image.open(crop / "inspection-plus2.png").convert("RGB")
        panels = [("original", original)]
        for method, score in scores.items():
            for threshold in (6, 10):
                raw = candidate_mask(score, threshold)
                for radius in (0, 2, 4):
                    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2 * radius + 1, 2 * radius + 1))
                    mask = cv2.dilate(raw.astype(np.uint8), kernel) > 0
                    rows.append({"case": case["name"], "kind": case["kind"], "method": method,
                                 "threshold": threshold, "dilation_radius": radius,
                                 "annotation_coverage": int((mask & support).sum()) / int(support.sum()),
                                 "annotation_pixels": int(support.sum()),
                                 "outside_annotation_flagged_pixels": int((mask & ~support).sum())})
                    if threshold == 10 and radius == 4:
                        panels.append((method + " 10,+4px", overlay(np.asarray(original), mask)))
        filename = case["name"] + ".png"
        make_sheet(panels, args.output / filename, 256)
        sections.append(f"<h2>{html.escape(case['name'])} — {html.escape(case['kind'])}</h2>"
                        f"<p>{html.escape(case.get('notes', ''))}</p><a href='{html.escape(filename, quote=True)}'>"
                        f"<img src='{html.escape(filename, quote=True)}' style='max-width:100%'></a>")
    (args.output / "measurements.json").write_text(json.dumps(rows, indent=2))
    (args.output / "index.html").write_text("<!doctype html><meta charset='utf-8'><title>Real-mark detection inspection</title>"
        "<body style='background:#222;color:#ddd;font:16px system-ui'><h1>Real-mark detection inspection</h1>"
        "<p>Visually labelled native crops, selected using Gaussian proposals: NOT an independent recall/precision benchmark. "
        "Annotation masks include repair margins. Pixels flagged outside annotations include unlabelled dust and scene detail; "
        "they are NOT measured false positives. Shared numerical thresholds are not equivalent across methods.</p>"
        "<p>Gaussian-positive tests only increased negative-log density, consistent with transmission loss. "
        "It is a physical-polarity hypothesis for these scans, not a universal dust detector. "
        "Overlays use score 10 followed by 4-pixel dilation. Full 6/10 and 0/2/4-pixel measurements are in measurements.json.</p>"
        + "".join(sections))


def report(args):
    args.output.mkdir(parents=True, exist_ok=False)
    rows = json.loads((args.repairs / "measurements.json").read_text())
    summary = []
    for method in REPAIRS:
        measured = [r for r in rows if r["method"] == method and r["kind"] == "held-out"]
        if not measured:
            continue
        summary.append(f"<tr><td>{method}</td><td>{len(measured)}</td>"
                       f"<td>{np.median([r['masked_highpass_ratio'] for r in measured]):.2f}</td>"
                       f"<td>{np.median([r['repair_seconds'] for r in measured]) * 1000:.1f}</td></tr>")
    def link(path):
        return html.escape(os.path.relpath(path, args.output), quote=True)
    sections = []
    unique = {r["case"]: r for r in rows}
    exposures = sorted({json.loads((args.repairs / case / "case.json").read_text())["inspection_ev"] for case in unique})
    for case, row in unique.items():
        image = link(args.repairs / case / "detail.png")
        sections.append(f"<h3>{html.escape(case)} — {html.escape(row['kind'])}</h3>"
                        f'<a href="{image}"><img src="{image}" alt="Repair comparison"></a>')
    for directory in sorted(args.detection.iterdir()):
        if directory.is_dir():
            image = link(directory / "comparison.png")
            sections.append(f"<h3>Detection: {html.escape(directory.name)}</h3>"
                            f'<a href="{image}"><img src="{image}" alt="Detection masks"></a>')
    content = """<!doctype html><meta charset="utf-8"><title>Dust algorithm experiments</title>
<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}a{color:#8cf}img{max-width:100%;image-rendering:auto}table{border-collapse:collapse}td,th{padding:8px;border:1px solid #666}</style>
<h1>Exploratory dust detection and repair</h1>
<p>Private native-resolution sample crops. Click images to inspect full-size detail comparisons.
Repair columns: original, fixed mask, harmonic, conditioned Telea, Navier–Stokes, donor clone, donor heal.</p>
<p><b>Not a validated dust benchmark.</b> Visual dust labels are inspection annotations, not an independently sampled detection benchmark. Unverified blemishes are not confirmed dust; held-out cases hide existing source pixels, not real defects.
Photographic calibration is diagnostic. Previews use the production ACES/sRGB renderer with the recorded inspection exposure, without changing working data.</p>
<p>Highpass ratio measures fine-frequency energy, not grain fidelity or semantic correctness. Timings exclude RAW development/rendering and do not establish interactive performance.</p>
<table><tr><th>Method</th><th>Held-out cases</th><th>Median highpass ratio</th><th>Median repair ms</th></tr>"""
    content += "".join(summary) + "</table>"
    content += f"<p>Inspection EV values: {html.escape(str(exposures))}. Table contains held-out cases only.</p>"
    content += "".join(sections)
    (args.output / "index.html").write_text(content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command, function in (("detect", detect), ("repair", repairs), ("synthetic-detect", synthetic_detection),
                              ("labelled-detect", labelled_detection)):
        p = sub.add_parser(command)
        p.add_argument("--crops", type=Path, required=True)
        p.add_argument("--output", type=Path, required=True)
        if command != "detect":
            p.add_argument("--cases", type=Path, required=True)
        if command in ("synthetic-detect", "labelled-detect"):
            p.add_argument("--baseline-scores", type=Path, required=True)
        if command == "repair":
            p.add_argument("--probe", type=Path, required=True)
            p.add_argument("--exposure", type=float, default=2)
        p.set_defaults(function=function)
    p = sub.add_parser("report")
    p.add_argument("--detection", type=Path, required=True)
    p.add_argument("--repairs", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.set_defaults(function=report)
    args = parser.parse_args()
    args.function(args)


if __name__ == "__main__":
    main()
