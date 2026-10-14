#!/usr/bin/env python3
"""Generate pixel-independent held-out repair cases from native crop dimensions."""
import argparse
import hashlib
import json
from pathlib import Path


def coordinate(data, offset, low, high):
    if high <= low:
        raise ValueError('Crop is too small for bounded validation cases')
    return low + int.from_bytes(data[offset:offset + 2], 'big') % (high - low)


def cases_for_crop(name, width, height, prefix):
    digest = hashlib.sha256(('antitone-dust-heldout-v1:' + name).encode()).digest()
    margin = 160
    centers = [(coordinate(digest, 0, margin, width - margin), coordinate(digest, 2, margin, height - margin)),
               (coordinate(digest, 4, margin, width - margin), coordinate(digest, 6, margin, height - margin))]
    directions = ((32, 0), (23, 23), (0, 32), (23, -23))
    dx, dy = directions[digest[8] % len(directions)]
    common = {'crop': str(Path(prefix) / name), 'kind': 'held-out',
              'selection': 'Pixel-independent SHA-256 coordinates fixed before running repair variants',
              'notes': 'Deterministic held-out source reconstruction, not a real defect or physically clean reference.'}
    circle = {**common, 'name': name + '-validation-circle',
              'shapes': [{'kind': 'circle', 'center': list(centers[0]), 'radius': 8}],
              'zoom': [centers[0][0] - 64, centers[0][1] - 64, centers[0][0] + 64, centers[0][1] + 64]}
    line = {**common, 'name': name + '-validation-line',
            'shapes': [{'kind': 'line', 'start': [centers[1][0] - dx, centers[1][1] - dy],
                        'end': [centers[1][0] + dx, centers[1][1] + dy], 'width': 7}],
            'zoom': [centers[1][0] - 64, centers[1][1] - 64, centers[1][0] + 64, centers[1][1] + 64]}
    return [circle, line]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--crop-root', type=Path, required=True)
    parser.add_argument('--crop', action='append', required=True)
    parser.add_argument('--case-prefix', default='crops')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    cases = []
    for name in args.crop:
        info = json.loads((args.crop_root / name / 'source.json').read_text())
        if info['stride'] != 1:
            raise ValueError('Validation requires native-resolution crops')
        cases.extend(cases_for_crop(name, info['width'], info['height'], args.case_prefix))
    with args.output.open('x') as output:
        json.dump(cases, output, indent=2)
    print(f'{len(cases)} pixel-independent held-out cases')


if __name__ == '__main__':
    main()
