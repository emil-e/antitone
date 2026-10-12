"""Capture only Antitone's window during viewer-tool or flat-field layout holds."""
import argparse
import os
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--binary', default='target/debug/antitone')
parser.add_argument('--samples', type=Path, default=Path('samples'))
parser.add_argument('--output', type=Path, default=Path('.cache/probes/ui-polish/viewer-tools'))
parser.add_argument('--flat-only', action='store_true', help='Capture the real flat-field reference layout without accepting a map or exporting')
options = parser.parse_args()
options.output.mkdir(parents=True, exist_ok=True)
log = options.output / 'smoke.txt'
if options.flat_only:
    args = [options.binary, '--smoke-flat-field', str(options.samples / 'DSC01737.ARW'), str(options.samples / 'DSC01739.ARW')]
    stages = ['flat-normal', 'flat-narrow']
    env = dict(os.environ, ANTITONE_SMOKE_FLAT_LAYOUT='1')
else:
    args = [options.binary, '--smoke', str(options.samples / 'DSC04397.ARW'), '4000', '5644', '128', '32', str(options.samples / 'DSC04378.ARW'), '4000', '2600', '128', '128', str(options.samples / 'DSC04416.ARW')]
    stages = ['crop-normal', 'dmin-normal', 'wb-normal', 'crop-narrow', 'dmin-narrow', 'wb-narrow']
    env = dict(os.environ, ANTITONE_SMOKE_TOOLS='1')
captured = set()
with log.open('w') as out:
    app = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, env=env)
    while app.poll() is None:
        text = log.read_text()
        for stage in stages:
            if f'tools_layout stage={stage}' not in text or stage in captured:
                continue
            window = subprocess.run(['.cache/probes/controls-window-id', str(app.pid)], capture_output=True, text=True)
            if window.returncode:
                continue
            capture = subprocess.run(['/usr/sbin/screencapture', '-x', '-o', '-l', window.stdout.strip(), str(options.output / f'{stage}.png')], capture_output=True, text=True)
            if capture.returncode:
                print(f'{stage}: capture failed: {capture.stderr.strip()}')
                app.terminate()
                app.wait()
                raise SystemExit(capture.returncode)
            captured.add(stage)
            print(f'{stage}: app-window capture saved')
        time.sleep(.1)
    code = app.wait()
print(f'Viewer tools smoke exit: {code}; captured {len(captured)}/{len(stages)} stages')
raise SystemExit(code or (0 if len(captured) == len(stages) else 1))
