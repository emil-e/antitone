#!/usr/bin/env python3
"""Fixed-mask donor eligibility, structural matching and correction ablations."""
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


METHODS = (("baseline", "donor-heal"), ("local", "donor-heal-local"),
           ("structure", "donor-heal-structure"), ("biharmonic", "donor-heal-refined"),
           ("boundary", "donor-heal-boundary"), ("ranked", "donor-heal-ranked"))


def reconstruction_metrics(truth, result, mask):
    metrics = e.measure_repair(truth, result, mask)
    def gradients(image):
        smooth = cv2.GaussianBlur(image, (7, 7), 1)
        return np.concatenate((cv2.Sobel(smooth, cv2.CV_32F, 1, 0, ksize=3, scale=1 / 8),
                               cv2.Sobel(smooth, cv2.CV_32F, 0, 1, ksize=3, scale=1 / 8)), axis=2)
    metrics['masked_structure_gradient_rmse'] = float(np.sqrt(np.mean((gradients(truth)[mask] - gradients(result)[mask]) ** 2)))
    for label, image in (('reference', truth), ('repaired', result)):
        highpass = image - cv2.GaussianBlur(image, (0, 0), 1)
        values = highpass[mask].astype(np.float64)
        centered = values - values.mean(axis=0)
        metrics[label + '_highpass_rgb_covariance'] = (centered.T @ centered / len(centered)).tolist()
    return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--crops', type=Path, required=True)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--domains', nargs='+', choices=('ap1', 'log-camera'), default=('ap1', 'log-camera'))
    parser.add_argument('--exposure', type=float, default=2)
    args = parser.parse_args()
    cases = json.loads(args.cases.read_text())
    names = [case['name'] for case in cases]
    if len(names) != len(set(names)):
        raise ValueError('Case names must be unique')
    args.output.mkdir(parents=True, exist_ok=False)
    rows, sections = [], []
    for case in cases:
        crop = args.crops / case['crop']
        info, camera, working = e.load_crop(crop)
        mask = (np.asarray(Image.open(case['mask_file']).convert('L')) > 0 if 'mask_file' in case
                else e.mask_from_shapes(working.shape, case['shapes']))
        if mask.shape != working.shape[:2] or not mask.any() or mask.all():
            raise ValueError('Invalid study mask')
        out = args.output / case['name']
        out.mkdir()
        Image.fromarray(mask.astype(np.uint8) * 255).save(out / 'mask.png')
        (out / 'case.json').write_text(json.dumps({'case': case, 'source': info, 'inspection_ev': args.exposure}, indent=2))
        original = e.render(args.probe, working, out, 'original', args.exposure)
        variants = [(name, method, None) for name, method in METHODS]
        if 'donor_offset' in case:
            variants += [('fixed-' + name, method, case['donor_offset'])
                         for name, method in METHODS if name not in ('local', 'ranked')]
        for domain in args.domains:
            panels = [('original', original.crop(case['zoom'])),
                      ('fixed mask', e.overlay(np.asarray(original), mask).crop(case['zoom']))]
            matched = {}
            for name, method, offset in variants:
                start = time.perf_counter()
                if domain == 'log-camera':
                    repaired, donors = stages.density_heal(camera, mask, donor_offset=offset, method=method)
                else:
                    repaired, donors = e.repair(working, mask, method, donor_offset=offset)
                seconds = time.perf_counter() - start
                prefix = domain + '-' + name
                if domain == 'log-camera':
                    result, floor = stages.develop(args.probe, crop, repaired, mask, out, prefix)
                    np.save(out / (prefix + '-camera-values.npy'), repaired[mask])
                else:
                    result, floor = repaired, None
                if not np.isfinite(result).all() or not e.outside_bit_exact(result, working, mask):
                    raise AssertionError('Refinement changed known AP1 or produced nonfinite output')
                positions = [(d['target'], d['source']) for d in donors]
                if name in ('structure', 'fixed-structure'):
                    matched[name] = positions
                if name in ('biharmonic', 'fixed-biharmonic', 'boundary', 'fixed-boundary'):
                    key = 'fixed-structure' if offset is not None else 'structure'
                    if positions != matched[key]:
                        raise AssertionError('Correction ablation changed donors')
                np.save(out / (prefix + '-masked-values.npy'), result[mask])
                image = e.render(args.probe, result, out, prefix, args.exposure)
                panels.append((name, image.crop(case['zoom'])))
                stages.donor_diagram(original, mask, donors, out / (prefix + '-donors.png'))
                row = {'case': case['name'], 'kind': case['kind'], 'domain': domain,
                       'method': name, 'engine': method, 'donor_offset': offset, 'donors': donors,
                       'mask_pixels': int(mask.sum()), 'repair_seconds': seconds,
                       'outside_bit_exact': True, 'floor': floor}
                if case['kind'] == 'held-out':
                    row.update(reconstruction_metrics(working, result, mask))
                rows.append(row)
            filename = domain + '-detail.png'
            e.make_sheet(panels, out / filename, 256)
            escaped = html.escape(case['name'], quote=True)
            sections.append(f"<h2>{escaped} — {html.escape(case['kind'])} — {domain}</h2>"
                            f"<p>{html.escape(case.get('notes', ''))}</p>"
                            f"<a href='{escaped}/{filename}'><img src='{escaped}/{filename}'></a>")
        print(case['name'], flush=True)
    (args.output / 'measurements.json').write_text(json.dumps(rows, indent=2))
    (args.output / 'index.html').write_text("<!doctype html><meta charset='utf-8'><title>Donor healing refinements</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px}img{max-width:100%}a{color:#8cf}</style>"
        "<h1>Donor healing ablations</h1><p>Baseline: original RGB matcher and whole-rectangle exclusion. "
        "Local: paired RGB context, source exclusion only for copied pixels and a two-pixel exterior. "
        "Structure: equal-weight contrast-normalized RGB and smoothed RGB gradients, not a classifier. "
        "Biharmonic: identical structural donors with a 13-point bilaplacian correction. Boundary: the same "
        "donors with eight stable biharmonic steps starting from harmonic correction, rather than an exact solve. "
        "Ranked: boundary correction plus distance-weighted context, a 3:1 colour/gradient mixture and nearest-source "
        "preference among donors within one percent of the best score. Nearby dependent mask "
        "components are jointly matched/solved. All methods retain the identical binary output support; no feather, "
        "clipping, synthetic grain or neural model. Source offsets never use hidden reference values.</p>"
        "<p>AP1 repairs follow the production pipeline. Log-camera repairs precede the unchanged selected-only "
        "development; known AP1 bytes remain exact. Diagnostic calibration and +2 EV viewing are not photographic "
        "acceptance. Real/ambiguous marks have no clean truth. Held-out RMSE, smoothed-gradient error and highpass "
        "RGB covariance measure different aspects of reconstruction, not grain fidelity or Lightroom parity. "
        "Timings exclude development, rendering and I/O. Click a comparison for full-size detail.</p>" + ''.join(sections))
    print(f'{len(rows)} comparisons; known AP1 bit patterns preserved')


if __name__ == '__main__':
    main()
