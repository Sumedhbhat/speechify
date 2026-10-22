"""Measure resident RAM and disk bytes for actual benchmark processes.

Use system Python: the orchestration process and its memory are excluded.
Model/backend matches run.py. No UI/clipboard action is performed.
"""
import argparse
import array
import datetime
import json
import os
from pathlib import Path
import random
import subprocess
import tempfile
import threading
import time
import wave

ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent

def memory(pid):
    fields={}
    try:
        for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
            split=line.split()
            if len(split)>=3 and split[1].isdigit():fields[split[0].rstrip(':')]=int(split[1])*1024
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith(('VmHWM:','VmRSS:')):
                k,v,*_=line.split();fields[k.rstrip(':')]=int(v)*1024
    except (ProcessLookupError,FileNotFoundError):return None
    return {'rss_bytes':fields['Rss'],'pss_bytes':fields['Pss'],'private_bytes':sum(fields.get(k,0) for k in ('Private_Clean','Private_Dirty','Private_Hugetlb')),'peak_rss_bytes':fields.get('VmHWM',0)}

def disk_union(paths):
    # Logical regular-file sizes and allocated bytes, deduplicated by inode.
    # Symlinks are not followed; reflink sharing is not detectable here.
    seen=set();logical=allocated=count=0
    for path in map(Path,paths):
        candidates=[path] if path.is_file() else path.rglob('*')
        for f in candidates:
            if f.is_symlink() or not f.is_file():continue
            stat=f.stat();identity=(stat.st_dev,stat.st_ino)
            if identity in seen:continue
            seen.add(identity);logical+=stat.st_size;allocated+=stat.st_blocks*512;count+=1
    return {'logical_bytes':logical,'allocated_bytes':allocated,'files':count}

def disk(path):
    return {'path':str(path),**disk_union([path])}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--engine-python',required=True)
    p.add_argument('--app-python',required=True)
    p.add_argument('--hf-home',required=True)
    args=p.parse_args()
    result={'metadata':{'date_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'sample_interval_s':0.1,'repeats_per_runtime':3,'worker':'shared, all three fixtures pre-warmed before memory measurement','clipboard_ui':'excluded','host_startup':'process launch to protocol-ready, model already warm; does not include full application startup','host_fixture_loading':'all three audio fixtures preloaded by both hosts','python_environment':'production dependency environment with evdev omitted; no microphone/hotkeys in benchmark'},'host_runs':[],'disk':{}}
    env={**os.environ,'PYTHONPATH':str(ROOT/'src'),'PYTHONUNBUFFERED':'1','HF_HOME':args.hf_home,'HF_HUB_OFFLINE':'1','OMP_NUM_THREADS':'1','OPENBLAS_NUM_THREADS':'1','MKL_NUM_THREADS':'1','SPEECHIFY_WORKER':json.dumps({'backend':'onnx','repo':'nemo-parakeet-tdt-0.6b-v3','opts':{'device':'cpu','compute_type':'int8','language':'en','beam_size':5}})}
    with tempfile.TemporaryDirectory(prefix='speechify-resources-') as temp:
        temp=Path(temp)
        with wave.open(str(ROOT/'src/speechify/selftest_data/librispeech-sample.wav')) as w:
            assert w.getnchannels()==1 and w.getframerate()==16000 and w.getsampwidth()==2
            ints=array.array('h',w.readframes(w.getnframes()))
        sample=array.array('f',(v/32768 for v in ints)); all_data=sample.tobytes()
        lengths={'short_3s':48000,'sample':len(sample),'long_30s':480000}
        buffers={name:(all_data*((n*4+len(all_data)-1)//len(all_data)))[:n*4] for name,n in lengths.items()}
        manifest={}
        for name,b in buffers.items():
            file=temp/f'{name}.f32';file.write_bytes(b);manifest[name]=str(file)
        (temp/'manifest.json').write_text(json.dumps(manifest))
        start=time.perf_counter_ns()
        worker=subprocess.Popen([args.engine_python,'-m','speechify.engines.worker'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE)
        ready=json.loads(worker.stdout.readline());assert ready['event']=='ready',ready
        result['model_startup_ns']=time.perf_counter_ns()-start;result['model_ready_info']=ready
        def request(name):
            worker.stdin.write(json.dumps({'samples':lengths[name],'prompt':None}).encode()+b'\n');worker.stdin.write(buffers[name]);worker.stdin.flush()
            reply=json.loads(worker.stdout.readline());assert reply.get('text')
            return reply['text']
        reference={name:request(name) for name in lengths}
        result['worker_warm_idle']=memory(worker.pid)
        order=['Python','Rust']*3;random.Random(20261022).shuffle(order)
        try:
            for index,runtime in enumerate(order):
                profile=temp/f'profile-{index}'
                if runtime=='Python':command=[args.app_python,str(HERE/'python_host.py'),str(worker.stdin.fileno()),str(worker.stdout.fileno()),str(temp/'manifest.json'),str(profile)]
                else:command=[str(HERE/'target/release/speechify-dictation-bench'),str(worker.stdin.fileno()),str(worker.stdout.fileno()),str(temp/'manifest.json')]
                start=time.perf_counter_ns()
                host=subprocess.Popen(command,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,pass_fds=(worker.stdin.fileno(),worker.stdout.fileno()))
                assert json.loads(host.stdout.readline())['ready']
                entry={'runtime':runtime,'order':index+1,'host_startup_ns':time.perf_counter_ns()-start,'host_idle':memory(host.pid),'worker_idle':memory(worker.pid),'requests':[],'samples':[]}
                stop=threading.Event()
                def sample_memory():
                    while not stop.is_set():
                        h,w=memory(host.pid),memory(worker.pid)
                        if h and w:entry['samples'].append({'host':h,'worker':w,'combined_rss_bytes':h['rss_bytes']+w['rss_bytes'],'combined_pss_bytes':h['pss_bytes']+w['pss_bytes']})
                        stop.wait(0.1)
                thread=threading.Thread(target=sample_memory);thread.start()
                try:
                    for name in lengths:
                        host.stdin.write(json.dumps({'fixture':name})+'\n');host.stdin.flush()
                        reply=json.loads(host.stdout.readline());assert reply['text']==reference[name]
                        entry['requests'].append({'fixture':name,**reply})
                    entry['host_after']=memory(host.pid);entry['worker_after']=memory(worker.pid)
                finally:
                    stop.set();thread.join();host.stdin.close();host.wait(timeout=10)
                result['host_runs'].append(entry)
                (HERE/'resources.json').write_text(json.dumps(result,indent=2)+'\n')
                print(runtime,'startup_ms',round(entry['host_startup_ns']/1e6,3),'host_idle_MiB',round(entry['host_idle']['pss_bytes']/2**20,2),'combined_peak_PSS_MiB',round(max(s['combined_pss_bytes'] for s in entry['samples'])/2**20,2),flush=True)
        finally:
            worker.stdin.close();worker.wait(timeout=10)
    paths={
        'app_source':ROOT/'src/speechify',
        'app_python_env':Path(args.app_python).parent.parent,
        'engine_python_env':Path(args.engine_python).parent.parent,
        'shared_python_distribution':Path(args.app_python).resolve().parent.parent,
        'model_cache':Path(args.hf_home),
        'rust_prototype_executable':HERE/'target/release/speechify-dictation-bench',
        'benchmark_rust_build_outputs':HERE/'target',
    }
    result['disk']={name:disk(path) for name,path in paths.items()}
    result['installed_stack_totals']={}
    for label,keys in {
        'Python':['app_source','app_python_env','engine_python_env','shared_python_distribution','model_cache'],
        'Rust_host_with_Python_worker':['app_source','rust_prototype_executable','engine_python_env','shared_python_distribution','model_cache'],
    }.items():
        result['installed_stack_totals'][label]={'components':keys,**disk_union([paths[k] for k in keys])}
    result['recording_growth']={'wav_bytes_per_second':32000,'wav_header_bytes_per_recording':44,'decimal_MB_per_minute':1.92,'decimal_MB_per_hour':115.2,'note':'16 kHz mono PCM16, keep_all_audio=true. Excludes SQLite/filesystem overhead.'}
    result['sampling_note']='One shared worker retains state; three observations per host, not an isolated full-app memory A/B. Host import timing and PSS depend on OS cache/shared pages. Combined differences include worker pool/state variation.'
    result['disk_note']='Rows are separate directory/file measurements; do not blindly sum overlapping or hard-linked paths. Logical bytes do not count followed symlink targets. Model cache includes actual blobs and metadata. Source row includes bundled selftest WAV. Rust executable is a host prototype, not a full feature-equivalent app.'
    (HERE/'resources.json').write_text(json.dumps(result,indent=2)+'\n')
    print('Disk:',json.dumps(result['disk'],indent=2),flush=True)

if __name__=='__main__':main()
