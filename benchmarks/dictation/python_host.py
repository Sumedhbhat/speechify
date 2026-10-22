"""Fresh-process Python dictation host for memory/startup measurements."""
import json
import os
from pathlib import Path
import sys
import time
from types import SimpleNamespace

import numpy as np
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'src'))
from speechify.audio import is_silent
from speechify.engines.host import EngineHost
from speechify.learning import Learner

write_fd,read_fd,manifest_path,profile=sys.argv[1:]
fixtures={k:np.fromfile(path,dtype='<f4') for k,path in json.loads(Path(manifest_path).read_text()).items()}
chunks={k:[v[i:i+480].copy() for i in range(0,len(v),480)] for k,v in fixtures.items()}
del fixtures
worker=SimpleNamespace(stdin=os.fdopen(int(write_fd),'wb'),stdout=os.fdopen(int(read_fd),'rb'))
engine=EngineHost();engine._proc=worker;engine.ready.set()
learner=Learner(Path(profile))
print(json.dumps({'ready':True}),flush=True)
for command in sys.stdin:
    name=json.loads(command)['fixture']
    start=time.perf_counter_ns()
    audio=np.concatenate(chunks[name]);assert not is_silent(audio)
    text,model_ms=engine.transcribe(audio,None)
    text=learner.apply(text)
    print(json.dumps({'runtime':'Python','text':text,'model_ms':model_ms,'ready_ns':time.perf_counter_ns()-start}),flush=True)
