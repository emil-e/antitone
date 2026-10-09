import os
import subprocess
import time
from pathlib import Path

log = Path('.cache/probes/film-model-images/response-controls-smoke.txt')
env = dict(os.environ, ANTITONE_SMOKE_CONTROLS='1')
args = ['target/release/antitone', '--smoke', 'samples/DSC04397.ARW', '4000', '5644', '128', '32', 'samples/DSC04378.ARW', '4000', '2600', '128', '128', 'samples/DSC04416.ARW']
captured = set()
with log.open('w') as out:
    app = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, env=env)
    while app.poll() is None:
        text = log.read_text()
        for stage in ['roll-normal', 'photo-normal', 'roll-narrow', 'photo-narrow']:
            if f'controls_layout stage={stage}' in text and stage not in captured:
                window = subprocess.run(['.cache/probes/controls-window-id', str(app.pid)], capture_output=True, text=True)
                if window.returncode == 0:
                    result = subprocess.run(['/usr/sbin/screencapture', '-x', '-o', '-l', window.stdout.strip(), f'.cache/probes/film-model-images/response-controls-{stage}.png'], capture_output=True, text=True)
                    print(f'{stage}: screenshot exit {result.returncode} {result.stderr.strip()}')
                else:
                    print(f'{stage}: window metadata unavailable')
                captured.add(stage)
        time.sleep(.1)
    code = app.wait()
print(f'Editor controls smoke exit: {code}, screenshot stages: {len(captured)}')
raise SystemExit(code)
