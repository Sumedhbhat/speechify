#!/usr/bin/env python3
"""Download data only; never import or execute dataset code. Python standard library."""
import hashlib
import json
from pathlib import Path
import random
import time
import urllib.request
import wave

ROOT = Path('/workspaces/.cache-speechify/eval')
REVISION = '00349cb980a38a68a1638f860e944a3bddabd5a7'
SEED = 20261111


def fetch(url, path):
    if path.exists():
        return path.read_bytes()
    for attempt in range(5):
        try:
            request_url = url + (('&' if '?' in url else '?') + f'eval_retry={attempt}' if attempt and 'datasets-server.huggingface.co/rows' in url else '')
            data = urllib.request.urlopen(request_url, timeout=30).read()
            path.parent.mkdir(parents=True, exist_ok=True)
            path.with_suffix(path.suffix + '.part').write_bytes(data)
            path.with_suffix(path.suffix + '.part').replace(path)
            return data
        except Exception:
            if attempt == 4:
                raise
            time.sleep(2)


def main():
    rows = []
    for offset in range(0, 6656, 100):
        url = ('https://datasets-server.huggingface.co/rows?dataset=Bhargav0044%2Fsvarah1'
               f'&config=default&split=train&offset={offset}&length=100')
        page = json.loads(fetch(url, ROOT / 'metadata' / f'{offset}.json'))
        rows.extend(page['rows'])
        print('metadata', len(rows), flush=True)
    assert len(rows) == 6656 and len({r['row_idx'] for r in rows}) == 6656
    assert all(REVISION in r['row']['audio'][0]['src'] for r in rows)
    eligible = [r for r in rows if 3 <= r['row']['duration'] <= 15]
    rng = random.Random(SEED)
    general = rng.sample(eligible, 60)
    selected = []
    for i, row in enumerate(general):
        selected.append((row, 'general', 'history' if i < 30 else 'test', ''))
    # Terms specified before any recognizer runs. Select four with most eligible
    # examples, excluding the random sample. No model-output-based selection.
    candidates = ['arunachal', 'pradesh', 'panchayat', 'ayushman', 'bharat',
                  'aadhaar', 'aadhar', 'karnataka', 'telangana', 'tamil',
                  'bengaluru', 'gujarat', 'maharashtra', 'odisha', 'hoysala',
                  'kerala', 'rajasthan', 'bihar', 'assam', 'sikkim']
    used = {r['row_idx'] for r in general}
    counts = {t: sum(t in r['row']['text'].lower().split() for r in eligible) for t in candidates}
    chosen = []
    for term in sorted(candidates, key=lambda t: (-counts[t], t)):
        pool = [r for r in eligible if r['row_idx'] not in used
                and term in r['row']['text'].lower().split()]
        if len(pool) < 6:
            continue
        group = rng.sample(pool, 6)
        chosen.append(term)
        for i, row in enumerate(group):
            selected.append((row, 'recurring', 'history' if i < 3 else 'test', term))
            used.add(row['row_idx'])
        if len(chosen) == 4:
            break
    manifest = []
    for row, scenario, split, term in selected:
        idx, data = row['row_idx'], row['row']
        path = ROOT / 'audio' / f'{idx}.wav'
        raw = fetch(data['audio'][0]['src'], path)
        with wave.open(str(path)) as wav:
            assert wav.getnchannels() == 1 and wav.getframerate() == 16000 and wav.getsampwidth() == 2
            duration = wav.getnframes() / 16000
        manifest.append(dict(id=str(idx), audio=str(path), reference=data['text'],
                             scenario=scenario, split=split, term=term, duration_s=duration,
                             sha256=hashlib.sha256(raw).hexdigest(), source_row=idx))
        print('audio', len(manifest), idx, flush=True)
        (ROOT / 'manifest.partial.json').write_text(json.dumps(manifest, indent=2))
    (ROOT / 'manifest.json').write_text(json.dumps(manifest, indent=2))
    cases = json.loads(Path(__file__).with_name('cases.json').read_text())
    (ROOT / 'cases-new.json').write_text(json.dumps([c for c in cases if c['source'] == 'new'], indent=2))
    (ROOT / 'selection.json').write_text(json.dumps(dict(seed=SEED, revision=REVISION,
        rows=len(rows), eligible=len(eligible), candidates=counts, chosen=chosen,
        general_n=60, recurring_n=len(manifest)-60), indent=2))


if __name__ == '__main__':
    main()
