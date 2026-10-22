"""Alternating before/after verification with the actual backend source.

Run with Python from the isolated ONNX environment, after the scheduling sweep.
Baseline comes from the recorded git revision, optimized code from this checkout.
Models never overlap in memory. No model/precision/decoder change is permitted.
"""
import argparse
import datetime
import hashlib
import importlib.metadata
import importlib.util
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import tempfile
import time

import numpy as np
ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
BASELINE='498358d0a18c7e23e1c49826a9383ad7aa3d097f'
sys.path.insert(0,str(ROOT/'src'))
from speechify.audio import load_wav


def memory():
    fields={}
    for line in Path('/proc/self/smaps_rollup').read_text().splitlines():
        parts=line.split()
        if len(parts)>=3 and parts[1].isdigit():fields[parts[0].rstrip(':')]=int(parts[1])*1024
    for line in Path('/proc/self/status').read_text().splitlines():
        if line.startswith(('VmHWM:','Threads:')):
            k,v,*_=line.split();fields[k.rstrip(':')]=int(v)*(1024 if k=='VmHWM:' else 1)
    return {k:fields.get(k,0) for k in ('Rss','Pss','Private_Clean','Private_Dirty','VmHWM','Threads')}


def child(variant,baseline_file,out):
    path=baseline_file if variant=='before' else ROOT/'src/speechify/engines/backends.py'
    spec=importlib.util.spec_from_file_location('measured_backends',path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    start=time.perf_counter_ns(); cpu=time.process_time_ns()
    engine=module.OnnxEngine('nemo-parakeet-tdt-0.6b-v3',{'device':'cpu','compute_type':'int8','language':'en','beam_size':5,'onnx_threads':4})
    load_ns=time.perf_counter_ns()-start;load_cpu_ns=time.process_time_ns()-cpu
    sample=load_wav(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')
    fixtures={'short_3s':sample[:48000],'sample':sample,'long_30s':np.resize(sample,480000)}
    engine.transcribe(np.zeros(16000,dtype=np.float32),None)
    for audio in fixtures.values():engine.transcribe(audio,None)
    sessions={name:{'intra_threads':s.get_session_options().intra_op_num_threads,'inter_threads':s.get_session_options().inter_op_num_threads,'per_session_threads':s.get_session_options().use_per_session_threads} for name,s in vars(engine.model.asr).items() if hasattr(s,'get_session_options')}
    result={'variant':variant,'load_ns':load_ns,'load_cpu_ns':load_cpu_ns,'warm_memory':memory(),'sessions':sessions,'measurements':[]}
    rng=random.Random(20261022)
    for repeat in range(3):
        names=list(fixtures);rng.shuffle(names)
        for name in names:
            start=time.perf_counter_ns();cpu=time.process_time_ns()
            text=engine.transcribe(fixtures[name],None)
            result['measurements'].append({'fixture':name,'repeat':repeat+1,'ns':time.perf_counter_ns()-start,'cpu_ns':time.process_time_ns()-cpu,'text':text})
    result['after_memory']=memory()
    out.write_text(json.dumps(result,indent=2)+'\n')


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--child',choices=['before','after']);p.add_argument('--baseline-file',type=Path);p.add_argument('--output',type=Path,default=HERE/'verification.json');a=p.parse_args()
    if a.child:return child(a.child,a.baseline_file,a.output)
    baseline=subprocess.check_output(['git','show',f'{BASELINE}:src/speechify/engines/backends.py'],cwd=ROOT)
    result={'metadata':{'date_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'baseline_commit':BASELINE,'baseline_source_sha256':hashlib.sha256(baseline).hexdigest(),'optimized_source_sha256':hashlib.sha256((ROOT/'src/speechify/engines/backends.py').read_bytes()).hexdigest(),'packages':{d.metadata['Name']:d.version for d in importlib.metadata.distributions()},'config':{'model':'nemo-parakeet-tdt-0.6b-v3','device':'cpu','compute_type':'int8','language':'en','onnx_threads_after':4},'order':['before','after','after','before','before','after'],'repeats_per_fixture_per_process':3,'warmup':'1 second of zeros, then all three fixtures once','model_revision':'8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce','fixture_sha256':hashlib.sha256((ROOT/'src/speechify/selftest_data/librispeech-sample.wav').read_bytes()).hexdigest()},'runs':[]}
    reference={}
    with tempfile.TemporaryDirectory(prefix='onnx-verify-') as tmp:
        baseline_file=Path(tmp)/'backends.py';baseline_file.write_bytes(baseline)
        for index,variant in enumerate(result['metadata']['order']):
            out=Path(tmp)/'result.json'
            subprocess.run([sys.executable,__file__,'--child',variant,'--baseline-file',str(baseline_file),'--output',str(out)],check=True,env={**os.environ,'HF_HUB_OFFLINE':'1','OMP_NUM_THREADS':'1','OPENBLAS_NUM_THREADS':'1','MKL_NUM_THREADS':'1'})
            run=json.loads(out.read_text());run['block']=index+1
            for m in run['measurements']:
                name=m['fixture']
                if name in reference:assert reference[name]==m['text'],(variant,name,reference[name],m['text'])
                reference[name]=m['text']
            result['runs'].append(run);a.output.write_text(json.dumps(result,indent=2)+'\n')
            print('block',index+1,variant,'warm PSS MiB',round(run['warm_memory']['Pss']/2**20,2),'threads',run['warm_memory']['Threads'],flush=True)
            for name in reference:
                rows=[r for r in run['measurements'] if r['fixture']==name]
                print(name,round(statistics.median(r['ns']/1e6 for r in rows),3),'ms',flush=True)

if __name__=='__main__':main()
