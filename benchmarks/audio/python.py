"""Production silence detection and the exact PCM expression from save_wav."""
import json
import sys
import time
from pathlib import Path
import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'src'))
from speechify.audio import is_silent

source, task, iterations, output = sys.argv[1:]
audio = np.fromfile(source, dtype='<f4')
def run():
    if task == 'silence':
        return is_silent(audio)
    return (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()
for _ in range(30):
    run()
start = time.perf_counter_ns()
for _ in range(int(iterations)):
    result = run()
ns = time.perf_counter_ns() - start
Path(output).write_bytes(bytes([result]) if task == 'silence' else result)
print(json.dumps({'ns_per_op': ns / int(iterations)}))
