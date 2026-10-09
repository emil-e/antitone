import subprocess, shlex, pathlib, sys
build = pathlib.Path('.cache/deps/ocio-build').resolve()
src = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'docs/research/tools/native/prophoto-internals.cpp').resolve()
cmd = shlex.split(subprocess.check_output(['ninja', '-C', str(build), '-t', 'commands', 'src/OpenColorIO/CMakeFiles/OpenColorIO.dir/ops/fixedfunction/ACES2/Transform.cpp.o'], text=True).splitlines()[-1])
output = pathlib.Path('.cache/probes') / src.stem
output.parent.mkdir(parents=True, exist_ok=True)
cmd = cmd[:cmd.index('-MD')] + ['-o', str(output.with_suffix('.o')), '-c', str(src)]
subprocess.run(cmd, check=True)
objs = [str(p) for p in (build / 'src/OpenColorIO/CMakeFiles/OpenColorIO.dir').rglob('*.o') if not str(p).endswith('ACES2/Transform.cpp.o')]
libs = [str(build / 'ext/dist/lib' / p) for p in ['libexpat.a', 'libImath-3_2.a', 'libpystring.a', 'libyaml-cpp.a', 'libminizip-ng.a', 'libz.a']]
subprocess.run(['/usr/bin/clang++', str(output.with_suffix('.o')), *objs, *libs, '-framework', 'ColorSync', '-framework', 'CoreFoundation', '-framework', 'CoreGraphics', '-framework', 'IOKit', '-o', str(output)], check=True)
with output.with_suffix('.txt').open('w') as out:
    subprocess.run([str(output.resolve())], stdout=out, check=True)
