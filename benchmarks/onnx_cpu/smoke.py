"""Check final backend code through the real persistent worker protocol."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'src'))
from speechify.audio import load_wav
from speechify.engines.host import EngineHost

python=sys.argv[1]
config={'backend':'onnx','repo':'nemo-parakeet-tdt-0.6b-v3','opts':{'device':'cpu','compute_type':'int8','language':'en','beam_size':5,'onnx_threads':4}}
env={**os.environ,'SPEECHIFY_WORKER':json.dumps(config),'PYTHONPATH':str(ROOT/'src'),'HF_HUB_OFFLINE':'1','PYTHONUNBUFFERED':'1','OMP_NUM_THREADS':'1','OPENBLAS_NUM_THREADS':'1','MKL_NUM_THREADS':'1'}
proc=subprocess.Popen([python,'-m','speechify.engines.worker'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,env=env)
engine=EngineHost();engine._proc=proc
try:
    ready=json.loads(proc.stdout.readline());assert ready['event']=='ready',ready
    engine.ready.set()
    audio=load_wav(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')
    start=time.perf_counter_ns();text,ms=engine.transcribe(audio,None);elapsed=time.perf_counter_ns()-start
    baseline=json.loads(Path(__file__).with_name('verification.json').read_text())
    expected=next(m['text'] for r in baseline['runs'] for m in r['measurements'] if m['fixture']=='sample')
    assert text==expected,(text,expected)
    result={'ready':ready,'text':text,'model_ms':ms,'host_ns':elapsed,'config':config,'backend_sha256':hashlib.sha256((ROOT/'src/speechify/engines/backends.py').read_bytes()).hexdigest(),'status':Path(f'/proc/{proc.pid}/status').read_text(),'note':'Final cleanup avoids setting ignored per-session thread counts when a global pool is selected; scheduling settings match verification. This is a protocol/parity smoke check, not an additional comparison sample.'}
    Path(__file__).with_name('smoke.json').write_text(json.dumps(result,indent=2)+'\n')
    print('Production worker protocol and transcript parity passed;',ms,'ms model')
finally:
    engine.stop()
