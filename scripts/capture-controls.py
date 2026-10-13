"""Capture six controls-layout holds from the release editor and named local RAWs.

Requires the compiled window locator, output directory and desktop capture permission.
Writes window PNGs/logs under .cache/probes/controls; missing stages or smoke failure fail.
Successful capture does not establish visual correctness or physical interaction.
"""
import os
import subprocess
import time
from pathlib import Path

log = Path('.cache/probes/controls/smoke.txt')
env = dict(os.environ, ANTITONE_SMOKE_CONTROLS='1')
args = ['target/release/antitone', '--smoke', 'samples/DSC04397.ARW', '4000', '5644', '128', '32', 'samples/DSC04378.ARW', '4000', '2600', '128', '128', 'samples/DSC04416.ARW']
stages = ['roll-normal', 'photo-normal', 'rendering-normal', 'roll-narrow', 'photo-narrow', 'rendering-narrow']
captured = set()
with log.open('w') as out:
    app = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, env=env)
    while app.poll() is None:
        text = log.read_text()
        for stage in stages:
            if f'controls_layout stage={stage}' in text and stage not in captured:
                window = subprocess.run(['.cache/probes/controls-window-id', str(app.pid)], capture_output=True, text=True)
                if window.returncode == 0:
                    result = subprocess.run(['/usr/sbin/screencapture', '-x', '-o', '-l', window.stdout.strip(), f'.cache/probes/controls/{stage}.png'], capture_output=True, text=True)
                    print(f'{stage}: screenshot exit {result.returncode} {result.stderr.strip()}')
                    if result.returncode == 0:
                        captured.add(stage)
                else:
                    print(f'{stage}: window metadata unavailable')
        time.sleep(.1)
    code = app.wait()
print(f'Editor controls smoke exit: {code}, screenshot stages: {len(captured)}/{len(stages)}')
raise SystemExit(code or (0 if len(captured) == len(stages) else 1))
