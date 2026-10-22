"""Attribute ASR wall time to native ONNX calls vs the rest of the backend.

Run after host timing, never concurrently with it. cProfile adds overhead;
these are diagnostic samples, not the primary latency comparison.
"""
import cProfile
import datetime
import hashlib
import importlib.metadata
import json
from pathlib import Path
import pstats
import sys
import time

import numpy as np
import onnxruntime as ort
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'src'))
from speechify.audio import load_wav
from speechify.engines.backends import OnnxEngine

# cProfile charges pybind native calls to the Python wrapper's self-time on
# this ORT build. Time the exact native-session boundary as well, so wrapper
# Python is not mislabeled native compute.
NATIVE_NS = 0
ORIGINAL_RUN = ort.InferenceSession.run
class TimedNativeSession:
    def __init__(self, inner): self.inner=inner
    def __getattr__(self, name): return getattr(self.inner,name)
    def run(self, *args, **kwargs):
        global NATIVE_NS
        start=time.perf_counter_ns()
        try: return self.inner.run(*args,**kwargs)
        finally: NATIVE_NS+=time.perf_counter_ns()-start

def measured_session_run(self, *args, **kwargs):
    if not isinstance(self._sess,TimedNativeSession): self._sess=TimedNativeSession(self._sess)
    return ORIGINAL_RUN(self,*args,**kwargs)
ort.InferenceSession.run=measured_session_run
out=Path(__file__).with_name('engine_profile.json')
start=time.perf_counter()
engine=OnnxEngine('nemo-parakeet-tdt-0.6b-v3',{'device':'cpu','compute_type':'int8','language':'en','beam_size':5})
load_s=time.perf_counter()-start
engine.transcribe(np.zeros(16000,dtype=np.float32),None)
sample=load_wav(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')
fixtures={'short_3s':sample[:48000],'sample':sample,'long_30s':np.resize(sample,480000)}
result={'date_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'load_s':load_s,'packages':{d.metadata['Name']:d.version for d in importlib.metadata.distributions()},'measurements':[]}
for repeat in range(3):
    for name,audio in fixtures.items():
        NATIVE_NS=0
        prof=cProfile.Profile(); start=time.perf_counter_ns(); prof.enable()
        text=engine.transcribe(audio,None)
        prof.disable(); elapsed=time.perf_counter_ns()-start
        stats=pstats.Stats(prof)
        entries=[{'filename':key[0],'line':key[1],'function':key[2],'primitive_calls':v[0],'calls':v[1],'self_s':v[2],'cumulative_s':v[3]} for key,v in stats.stats.items()]
        entries.sort(key=lambda e:e['self_s'],reverse=True)
        # Preserve all entries so the classification is auditable.
        record={'fixture':name,'repeat':repeat+1,'elapsed_ns':elapsed,'native_call_ns':NATIVE_NS,'profile_total_s':stats.total_tt,'text':text,'entries':entries}
        result['measurements'].append(record)
        out.write_text(json.dumps(result,indent=2)+'\n')
        print(name,repeat+1,elapsed/1e6,'ms',[(e['function'],round(e['self_s'],4)) for e in entries[:5]],flush=True)
# Hash actual resolved model contents and pin the Hub revision, not just a name.
home=Path(__import__('os').environ.get('HF_HOME',Path.home()/'.cache/huggingface'))
files=[]
for snapshot in (home/'hub').glob('models--istupakov--parakeet-tdt-0.6b-v3-onnx/snapshots/*'):
    for file in snapshot.iterdir():
        if file.is_file():
            files.append({'revision':snapshot.name,'name':file.name,'size':file.stat().st_size,'sha256':hashlib.file_digest(file.open('rb'),'sha256').hexdigest()})
result['model_files']=files
out.write_text(json.dumps(result,indent=2)+'\n')
