"""Run from repo root: .venv/bin/python benchmarks/audio/run.py --go /path/to/go.

No model inference, file I/O, process startup or compilation is timed.
"""
import argparse
import datetime
import hashlib
import json
import os
import platform
import random
import shutil
import shlex
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'src'))
from speechify.audio import load_wav


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', default=shutil.which('go'))
    parser.add_argument('--rounds', type=int, default=9)
    parser.add_argument('--out', type=Path, default=ROOT / 'benchmarks/audio/results.json')
    args = parser.parse_args()
    if not args.go:
        parser.error('Go is required: supply --go /path/to/go')
    env = {**os.environ, 'OMP_NUM_THREADS': '1', 'OPENBLAS_NUM_THREADS': '1', 'MKL_NUM_THREADS': '1', 'GOMAXPROCS': '1'}
    source = ROOT / 'benchmarks/audio'
    def execute(cmd):
        return subprocess.check_output([str(c) for c in cmd], cwd=ROOT, env=env, text=True).strip()
    with tempfile.TemporaryDirectory(prefix='speechify-bench-') as temp:
        temp = Path(temp)
        rust_extra = shlex.split(os.environ.get('BENCH_RUSTFLAGS', ''))
        execute(['rustc', *rust_extra, '--edition=2021', '-O', '-C', 'target-cpu=x86-64', source / 'rust.rs', '-o', temp / 'rust'])
        execute([args.go, 'build', '-o', temp / 'go', source / 'go.go'])
        commands = {
            'Python/NumPy': [sys.executable, source / 'python.py'],
            'Rust': [temp / 'rust'],
            'Go': [temp / 'go'],
            'TypeScript/Node': ['node', source / 'typescript.ts'],
            'TypeScript/Bun': ['bun', source / 'typescript.ts'],
        }
        fixture = ROOT / 'src/speechify/selftest_data/librispeech-sample.wav'
        audio = load_wav(fixture)
        results = {'metadata': {
            'measured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in source.iterdir() if p.suffix in ('.py','.rs','.go','.ts')},
            'platform': platform.platform(), 'cpu': next((x.split(':',1)[1].strip() for x in Path('/proc/cpuinfo').read_text().splitlines() if x.startswith('model name')), platform.processor()),
            'logical_cpus': os.cpu_count(), 'affinity': sorted(os.sched_getaffinity(0)),
            'python': sys.version, 'numpy': np.__version__,
            'rust': execute(['rustc','--version']), 'go': execute([args.go,'version']),
            'node': execute(['node','--version']), 'bun': execute(['bun','--version']),
            'commit': execute(['git','rev-parse','HEAD']),
            'fixture_sha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
            'rounds': args.rounds, 'warmup_iterations_per_process': 30,
            'sample_rate': 16000, 'channels': 1, 'dtype': 'little-endian float32',
            'threads': {k: env[k] for k in ('OMP_NUM_THREADS','OPENBLAS_NUM_THREADS','MKL_NUM_THREADS','GOMAXPROCS')},
            'rust_flags': '--edition=2021 -O -C target-cpu=x86-64 ' + ' '.join(rust_extra), 'go_flags': 'build (default optimizations)',
            'seed': 20261022,
        }, 'validation': [], 'measurements': []}
        # Include silence, clipping, sign, truncation and threshold cases.
        fixtures = {'edge': np.array([-2,-1,-0.5,-1/32767,0,1/32767,0.5,1,2],dtype='<f4'),
                    'empty': np.array([],dtype='<f4'), 'quiet': np.zeros(16000,dtype='<f4'),
                    'below': np.full(16000,0.0039,dtype='<f4'), 'above': np.full(16000,0.0041,dtype='<f4')}
        for seconds in (1,10,60):
            fixtures[str(seconds)] = np.resize(audio,seconds*16000).astype('<f4')
        for name, data in fixtures.items():
            file = temp / f'{name}.f32'; data.tofile(file)
            for task in ('silence','pcm16'):
                expected = temp / 'expected'; actual = temp / 'actual'
                execute([*commands['Python/NumPy'],file,task,1,expected])
                for runtime, cmd in commands.items():
                    execute([*cmd,file,task,1,actual])
                    if actual.read_bytes() != expected.read_bytes():
                        raise AssertionError(f'{runtime} mismatches Python for {name}/{task}')
                results['validation'].append({'fixture':name,'task':task,'bytes':expected.stat().st_size,'sha256':hashlib.sha256(expected.read_bytes()).hexdigest()})
        jobs = [(seconds,task,runtime) for seconds in (1,10,60) for task in ('silence','pcm16') for runtime in commands]
        rng = random.Random(20261022)
        for round_number in range(args.rounds):
            rng.shuffle(jobs)
            for seconds,task,runtime in jobs:
                iterations = max(100, 6000 // seconds)
                measured = json.loads(execute([*commands[runtime],temp/f'{seconds}.f32',task,iterations,temp/'actual']))
                results['measurements'].append({'round':round_number+1,'seconds':seconds,'task':task,'runtime':runtime,'iterations':iterations,**measured})
            print(f'Round {round_number+1}/{args.rounds} complete',flush=True)
        args.out.write_text(json.dumps(results,indent=2)+'\n')
        for seconds in (1,10,60):
            for task in ('silence','pcm16'):
                medians = {r:statistics.median(x['ns_per_op'] for x in results['measurements'] if x['seconds']==seconds and x['task']==task and x['runtime']==r) for r in commands}
                for runtime, ns in medians.items():
                    print(f'{seconds:2}s {task:7} {runtime:16} {ns/1000:10.3f} us {medians["Python/NumPy"]/ns:7.3f}x')

if __name__ == '__main__':
    main()
