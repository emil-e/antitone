#!/usr/bin/env python3
"""Prepare Antitone-produced TIFF inputs for a manual Lightroom Heal comparison."""
import argparse
import html
import json
from pathlib import Path
import subprocess

import cv2
import numpy as np
from PIL import Image

import experiment as e
from mask_study import score_support


ALGORITHM = 'ap1-boundary-ranked-score10-grow4-v1'


def export_tiff(probe, crop, working, output, temporary):
    working.astype('<f4').tofile(temporary)
    subprocess.run([str(probe), 'tiff', str(crop), str(temporary), str(output), 'srgb'],
                   check=True, capture_output=True, text=True)
    temporary.unlink()
    decoded = cv2.imread(str(output), cv2.IMREAD_UNCHANGED)
    if decoded is None or decoded.dtype != np.uint16 or decoded.shape != working.shape:
        raise AssertionError('Production TIFF did not reopen as RGB16 at source dimensions')
    with Image.open(output) as image:
        if not image.tag_v2.get(34675) or 'Antitone model 4' not in image.tag_v2.get(270, ''):
            raise AssertionError('Production TIFF is missing Antitone ICC/processing metadata')
    return decoded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--crops', type=Path, required=True)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--case', action='append', dest='selected')
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    cases = json.loads(args.cases.read_text())
    if args.selected:
        by_name = {case['name']: case for case in cases}
        missing = [name for name in args.selected if name not in by_name]
        if missing:
            raise ValueError('Unknown benchmark cases: ' + ', '.join(missing))
        cases = [by_name[name] for name in args.selected]
    names = [case['name'] for case in cases]
    if not names or len(names) != len(set(names)):
        raise ValueError('Benchmark case names must be nonempty and unique')
    args.output.mkdir(parents=True, exist_ok=False)
    inputs = args.output / 'inputs'
    references = args.output / 'antitone-reference'
    guides = args.output / 'guides'
    for directory in (inputs, references, guides):
        directory.mkdir()
    records, sections, results = [], [], []
    for case in cases:
        crop = args.crops / case['crop']
        info, camera, working = e.load_crop(crop)
        annotation = (np.asarray(Image.open(case['mask_file']).convert('L')) > 0 if 'mask_file' in case
                      else e.mask_from_shapes(working.shape, case['shapes']))
        score = e.positive_gaussian_score(camera)
        core, support = score_support(annotation, score, threshold=10, growth=4)
        repaired, donors = e.repair(working, support, 'donor-heal-ranked')
        if not e.outside_bit_exact(repaired, working, support):
            raise AssertionError('Benchmark repair changed pixels outside support')
        input_tiff = inputs / (case['name'] + '.tif')
        reference_tiff = references / (case['name'] + '-antitone.tif')
        input_codes = export_tiff(args.probe, crop, working, input_tiff,
                                  args.output / (case['name'] + '-input.ap1-f32'))
        reference_codes = export_tiff(args.probe, crop, repaired, reference_tiff,
                                      args.output / (case['name'] + '-repair.ap1-f32'))
        if not np.array_equal(input_codes[~support], reference_codes[~support]):
            raise AssertionError('Benchmark TIFF pair differs outside repair support')
        if np.array_equal(input_codes[support], reference_codes[support]):
            raise AssertionError('Benchmark repair made no encoded change inside support')
        original = e.render(args.probe, working, guides, case['name'] + '-input', 0)
        Image.fromarray(support.astype(np.uint8) * 255).save(guides / (case['name'] + '-mask.png'))
        guide = e.make_sheet([('input', original.crop(case['zoom'])),
                              ('repair support', e.overlay(np.asarray(original), support).crop(case['zoom']))],
                             guides / (case['name'] + '-guide.png'), 384)
        del guide
        offset = [(donor['source'][0] - donor['target'][0], donor['source'][1] - donor['target'][1])
                  for donor in donors]
        record = {'case': case['name'], 'kind': case['kind'], 'algorithm': ALGORITHM,
                  'input_tiff': str(input_tiff.relative_to(args.output)),
                  'antitone_reference_tiff': str(reference_tiff.relative_to(args.output)),
                  'guide': str((guides / (case['name'] + '-guide.png')).relative_to(args.output)),
                  'mask': str((guides / (case['name'] + '-mask.png')).relative_to(args.output)),
                  'dimensions': [info['width'], info['height']], 'zoom': case['zoom'],
                  'annotation_pixels': int(annotation.sum()), 'core_pixels': int(core.sum()),
                  'support_pixels': int(support.sum()), 'donor_offsets': offset,
                  'outside_support_rgb16_exact': True, 'notes': case.get('notes', '')}
        records.append(record)
        results.append({'case': case['name'], 'lightroom_version': None, 'heal_mode': 'Heal (non-generative)',
                        'default_success': None, 'source_moves': None, 'mask_edits': None,
                        'elapsed_seconds': None, 'export_file': case['name'] + '-lightroom.tif', 'notes': None})
        escaped = html.escape(case['name'], quote=True)
        sections.append(f"<h2>{escaped}</h2><p>{html.escape(case.get('notes', ''))}</p>"
                        f"<a href='guides/{escaped}-guide.png'><img src='guides/{escaped}-guide.png'></a>")
        print(case['name'], flush=True)
    (args.output / 'manifest.json').write_text(json.dumps({'algorithm': ALGORITHM,
        'input': 'Antitone model-4 AP1 crop snapshot through production ACES 2 sRGB RGB16 TIFF export; no inspection EV',
        'lightroom_mode': 'Heal (non-generative)', 'cases': records}, indent=2))
    (args.output / 'lightroom-results-template.json').write_text(json.dumps(results, indent=2))
    (args.output / 'index.html').write_text("<!doctype html><meta charset='utf-8'><title>Lightroom Heal benchmark</title>"
        "<style>body{background:#222;color:#ddd;font:16px system-ui;margin:24px;max-width:1200px}img{max-width:100%}a{color:#8cf}code{background:#333;padding:2px 4px}</style>"
        "<h1>Antitone → Lightroom Heal comparison</h1>"
        "<ol><li>Import only the RGB16 sRGB files under <code>inputs/</code>. They are production Antitone colour "
        "exports with dust intact and no inspection exposure.</li><li>Do not change tone, colour, crop, sharpening, "
        "noise reduction or geometry.</li><li>Use Lightroom <b>Heal</b>, not Generative Remove. Match the cyan support "
        "in each guide as closely as practical. Accept Lightroom's initial source first; record whether it succeeds "
        "before moving the source or editing the mask.</li><li>Export at original dimensions as 16-bit sRGB TIFF, "
        "without resizing or output sharpening, named <code>CASE-lightroom.tif</code>.</li><li>Fill in "
        "<code>lightroom-results-template.json</code>. Keep Lightroom outputs beside that file.</li></ol>"
        "<p>The files under <code>antitone-reference/</code> use the frozen Antitone repair and are for comparison "
        "after Lightroom work, not Lightroom inputs. These bounded f32 AP1 snapshots isolate repair while using the "
        "production TIFF renderer/encoder; they are not full-photo export or Lightroom interoperability acceptance. "
        "Real marks have no clean reconstruction truth.</p>" + ''.join(sections))
    print(f'{len(records)} RGB16 benchmark pairs; production sRGB ICC embedded')


if __name__ == '__main__':
    main()
