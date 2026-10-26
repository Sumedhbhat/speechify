"""Narrow exception: Kyutai, Distil-Whisper and trained CTranslate2 models.
No application lifecycle, microphone, keys, clipboard, settings or learning code.
"""
import os, sys, json, time
sys.path.insert(0, os.path.join(os.path.dirname(__file__), 'src'))
proto = os.fdopen(os.dup(1), 'w', buffering=1)
os.dup2(2, 1)
sys.stdout = sys.stderr
import numpy as np
from speechify.engines.backends import TransformersEngine, WhisperEngine
engine = None
for line in sys.stdin:
    try:
        msg = json.loads(line)
        if msg['type'] == 'load':
            if msg['model'] not in ('kyutai-stt-1b', 'kyutai-stt-2.6b', 'distil-whisper-large-v3') and not msg['model'].startswith('ft:'):
                raise ValueError('Python is restricted to unsupported native families and trained models')
            start = time.perf_counter()
            cls = TransformersEngine if msg['model'].startswith('kyutai') else WhisperEngine
            engine = cls(msg['repo'], msg['opts'])
            out = {'ready': True, 'device': getattr(engine, 'device', msg['opts']['device']), 'ms': (time.perf_counter()-start)*1000}
        elif msg['type'] == 'run':
            start = time.perf_counter()
            audio = np.array(msg['pcm'], dtype=np.float32)
            text, lp = engine.transcribe_scored(audio, msg.get('prompt'))
            out = {'text': text, 'confidence': float(np.exp(lp)) if lp is not None else None, 'device': getattr(engine, 'device', '?'), 'ms': (time.perf_counter()-start)*1000}
        else:
            raise ValueError('Unknown worker operation')
        proto.write(json.dumps({'id': msg['id'], **out})+'\n')
    except Exception as e:
        proto.write(json.dumps({'id': msg.get('id'), 'error': f'{type(e).__name__}: {e}'})+'\n')
