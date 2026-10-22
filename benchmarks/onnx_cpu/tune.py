"""Screen ONNX scheduling settings without changing model/precision/decoding.

Run with the isolated ONNX environment. Each variant loads in its own child
process, so sessions/weights/thread pools never overlap.
"""
import argparse
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time
import types

import numpy as np
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'src'))
from speechify.audio import load_wav

VARIANTS={'default':None,'threads1':(1,True),'threads2':(2,True),'threads4':(4,True),'threads2_no_spin':(2,False),'default_no_spin':(0,False),'threads4_no_spin':(4,False),'shared_auto':(0,True),'shared4':(4,True)}

def child(variant,output):
    import onnxruntime as ort
    import onnx_asr
    # Keep the sweep reproducible after the winning change is implemented.
    baseline = subprocess.check_output([
        'git', 'show', '498358d0a18c7e23e1c49826a9383ad7aa3d097f:src/speechify/engines/backends.py',
    ], cwd=ROOT)
    module = types.ModuleType('baseline_backends')
    exec(compile(baseline, 'baseline_backends.py', 'exec'), module.__dict__)
    OnnxEngine = module.OnnxEngine
    setting=VARIANTS[variant]
    if variant.startswith('shared'):
        ort.set_global_thread_pool_sizes(intra_op_num_threads=setting[0],inter_op_num_threads=1)
    if setting:
        original=onnx_asr.load_model
        def load(*args,**kwargs):
            options=ort.SessionOptions()
            if variant.startswith('shared'):
                options.use_per_session_threads=False
            else:
                options.intra_op_num_threads=setting[0]
                options.inter_op_num_threads=1
            if not setting[1]:
                options.add_session_config_entry('session.intra_op.allow_spinning','0')
                options.add_session_config_entry('session.inter_op.allow_spinning','0')
            return original(*args,**kwargs,sess_options=options)
        onnx_asr.load_model=load
    start=time.perf_counter_ns()
    model=OnnxEngine('nemo-parakeet-tdt-0.6b-v3',{'device':'cpu','compute_type':'int8','language':'en','beam_size':5})
    load_ns=time.perf_counter_ns()-start
    sample=load_wav(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')
    fixtures={'short_3s':sample[:48000],'sample':sample}
    model.transcribe(np.zeros(16000,dtype=np.float32),None)
    for audio in fixtures.values():model.transcribe(audio,None)
    result={'variant':variant,'load_ns':load_ns,'measurements':[]}
    rng=random.Random(20261022)
    for repeat in range(3):
        names=list(fixtures);rng.shuffle(names)
        for name in names:
            start=time.perf_counter_ns();cpu=time.process_time_ns()
            text=model.transcribe(fixtures[name],None)
            result['measurements'].append({'fixture':name,'repeat':repeat+1,'elapsed_ns':time.perf_counter_ns()-start,'cpu_ns':time.process_time_ns()-cpu,'text':text})
    result['status']=Path('/proc/self/status').read_text()
    output.write_text(json.dumps(result,indent=2)+'\n')

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--child',choices=VARIANTS);p.add_argument('--output',type=Path,default=Path(__file__).with_name('screen.json'));args=p.parse_args()
    if args.child:return child(args.child,args.output)
    results=[];reference={}
    for variant in VARIANTS:
        out=args.output.with_name(f'screen-{variant}.json')
        subprocess.run([sys.executable,__file__,'--child',variant,'--output',str(out)],check=True,env={**os.environ,'HF_HUB_OFFLINE':'1','OMP_NUM_THREADS':'1','OPENBLAS_NUM_THREADS':'1','MKL_NUM_THREADS':'1'})
        data=json.loads(out.read_text())
        for record in data['measurements']:
            name=record['fixture']
            if name in reference:assert record['text']==reference[name],(variant,name,record['text'],reference[name])
            reference[name]=record['text']
        results.append(data);args.output.write_text(json.dumps(results,indent=2)+'\n')
        for name in reference:
            rows=[r for r in data['measurements'] if r['fixture']==name]
            print(variant,name,'median_ms',round(statistics.median(r['elapsed_ns']/1e6 for r in rows),3),'cpu_ms',round(statistics.median(r['cpu_ns']/1e6 for r in rows),3),flush=True)

if __name__=='__main__':main()
