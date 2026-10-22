"""Compare the production Python host path to Rust using ONE shared ASR worker.

Linux-only FD handoff, no clipboard/UI actions. A fresh learning profile is used.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import random
import shlex
import statistics
import subprocess
import sys
import tempfile
import time

import numpy as np
ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'src'))
from speechify.audio import is_silent, load_wav
from speechify.config import Config
from speechify.engines.host import EngineHost
from speechify.learning import Learner


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine-python', required=True)
    parser.add_argument('--rounds', type=int, default=12)
    parser.add_argument('--out', type=Path, default=HERE / 'results.json')
    args = parser.parse_args()
    subprocess.run(['cargo','build','--release','--locked','--manifest-path',str(HERE/'Cargo.toml')],check=True)
    env = dict(os.environ)
    env.update(PYTHONPATH=os.pathsep.join([str(ROOT/'src'),str(HERE)]), PYTHONUNBUFFERED='1', OMP_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1', MKL_NUM_THREADS='1', HF_HUB_OFFLINE='1')
    config = {'backend':'onnx', 'repo':'nemo-parakeet-tdt-0.6b-v3','opts':{'device':'cpu','compute_type':'int8','language':'en','beam_size':5}}
    env['SPEECHIFY_WORKER'] = json.dumps(config)
    results = {'metadata':{
        'date_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'config':config,'profile':'fresh: no rules, empty glossary, LLM disabled',
        'rounds':args.rounds,'warmup_requests_per_fixture_runtime':2,'seed':20261022,
        'python':sys.version,'numpy':np.__version__,
        'rust':subprocess.check_output(['rustc','--version'],text=True).strip(),
        'cpu':next(x.split(':',1)[1].strip() for x in Path('/proc/cpuinfo').read_text().splitlines() if x.startswith('model name')),
        'affinity':sorted(os.sched_getaffinity(0)),
        'onnx_threading':'production library defaults, one shared worker; environment OMP/BLAS limits=1',
        'fixed_clipboard_delay_ms':20,'fixed_clipboard_delay':'modeled from output.py, not an actual paste measurement',
        'rustflags':os.environ.get('RUSTFLAGS',''),
        'packages':subprocess.check_output([args.engine_python,'-m','pip','freeze'],text=True).strip() if False else '',
        'fixture_sha256':hashlib.sha256((ROOT/'src/speechify/selftest_data/librispeech-sample.wav').read_bytes()).hexdigest(),
        'source_hashes':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [HERE/'run.py',HERE/'src/main.rs',ROOT/'src/speechify/audio.py',ROOT/'src/speechify/engines/worker.py',ROOT/'src/speechify/engines/backends.py',ROOT/'src/speechify/engines/host.py',ROOT/'src/speechify/output.py']},
    },'measurements':[],'startup':[]}
    with tempfile.TemporaryDirectory(prefix='dictation-') as temp:
        temp = Path(temp)
        sample=load_wav(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')
        # Full sample is the accuracy fixture; shorter prefix and repeated clip
        # provide differing audio/compute sizes using the same input source.
        fixtures={'short_3s':sample[:48000], 'sample':sample, 'long_30s':np.resize(sample,480000)}
        results['metadata']['fixture_seconds']={k:len(v)/16000 for k,v in fixtures.items()}
        manifest={}
        for name,audio in fixtures.items():
            path=temp/f'{name}.f32'; audio.astype('<f4').tofile(path); manifest[name]=str(path)
        (temp/'manifest.json').write_text(json.dumps(manifest))
        learner=Learner(temp/'profile')
        chunks={k:[v[i:i+480].copy() for i in range(0,len(v),480)] for k,v in fixtures.items()}
        reference={}
        for mode in ('real','no_inference_control'):
            worker_env=dict(env)
            if mode!='real':
                worker_env['SPEECHIFY_WORKER']=json.dumps({**config,'backend':'echo_engine:Echo'})
                worker_env['BENCH_ECHO_TEXT']=reference['sample']
            start=time.perf_counter_ns()
            proc=subprocess.Popen([args.engine_python,'-m','speechify.engines.worker'],env=worker_env,stdin=subprocess.PIPE,stdout=subprocess.PIPE)
            msg=json.loads(proc.stdout.readline())
            if msg.get('event')!='ready':
                raise RuntimeError(msg)
            results['startup'].append({'mode':mode,'ns':time.perf_counter_ns()-start,'info':msg})
            engine=EngineHost(); engine._proc=proc; engine.ready.set()
            rust=subprocess.Popen([str(HERE/'target/release/speechify-dictation-bench'),str(proc.stdin.fileno()),str(proc.stdout.fileno()),str(temp/'manifest.json')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,pass_fds=(proc.stdin.fileno(),proc.stdout.fileno()))
            assert json.loads(rust.stdout.readline())['ready']
            def run(runtime,name):
                if runtime=='Rust':
                    rust.stdin.write(json.dumps({'fixture':name})+'\n'); rust.stdin.flush()
                    return json.loads(rust.stdout.readline())
                start=time.perf_counter_ns()
                audio=np.concatenate(chunks[name])
                assert not is_silent(audio)
                audio_ns=time.perf_counter_ns()-start
                prompt=learner.prompt()
                ipc_start=time.perf_counter_ns()
                text,model_ms=engine.transcribe(audio,None)  # Parakeet has no glossary support.
                ipc_ns=time.perf_counter_ns()-ipc_start
                text=learner.apply(text)
                return {'runtime':'Python','text':text,'audio_ns':audio_ns,'ipc_ns':ipc_ns,'ready_ns':time.perf_counter_ns()-start,'model_ms':model_ms}
            try:
                for name in fixtures:
                    for runtime in ('Python','Rust'):
                        for _ in range(2):
                            record=run(runtime,name)
                            if mode=='real':
                                if name in reference: assert record['text']==reference[name],record
                                reference[name]=record['text']
                rng=random.Random(20261022)
                rounds=args.rounds if mode=='real' else max(args.rounds,30)
                for round in range(rounds):
                    names=list(fixtures); rng.shuffle(names)
                    for name in names:
                        runtimes=['Python','Rust']; rng.shuffle(runtimes)
                        for runtime in runtimes:
                            record=run(runtime,name)
                            assert record['text']==(reference[name] if mode=='real' else reference['sample'])
                            results['measurements'].append({'mode':mode,'fixture':name,'round':round+1,**record})
                    args.out.write_text(json.dumps(results,indent=2)+'\n')
                    if mode=='real' or (round+1)%10==0: print(f'{mode}: round {round+1}/{rounds} complete',flush=True)
            finally:
                rust.stdin.close(); rust.wait(timeout=10); engine.stop()
        results['transcripts']=reference
        args.out.write_text(json.dumps(results,indent=2)+'\n')
        for name in fixtures:
            for mode in ('real','no_inference_control'):
                rows=[r for r in results['measurements'] if r['fixture']==name and r['mode']==mode]
                for runtime in ('Python','Rust'):
                    selected=[r for r in rows if r['runtime']==runtime]
                    print(name,mode,runtime,'ready_ms',statistics.median(r['ready_ns']/1e6 for r in selected),'model_ms',statistics.median(r['model_ms'] for r in selected))

if __name__=='__main__': main()
