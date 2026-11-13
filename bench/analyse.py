#!/usr/bin/env python3
"""Independent standard-library corpus WER and paired clip bootstrap.

Neither core's alignment nor teacher transcripts are used as ground truth.
"""
import argparse
import json
import math
from pathlib import Path
import random
import statistics

SEED = 20261111
TRIALS = 10000


def normalize(text):
    return ''.join(c for c in text.lower() if c.isalnum() or c.isspace()).split()


def distance(reference, hypothesis):
    prev = list(range(len(hypothesis) + 1))
    for i, a in enumerate(reference, 1):
        row = [i]
        for j, b in enumerate(hypothesis, 1):
            row.append(min(row[-1] + 1, prev[j] + 1, prev[j-1] + (a != b)))
        prev = row
    return prev[-1]


def errors(reference, hypothesis):
    a, b = normalize(reference), normalize(hypothesis)
    return distance(a, b), len(a)


def percentile(values, p):
    return sorted(values)[max(0, math.ceil(len(values) * p) - 1)]


def ratio(pairs):
    return sum(x[0] for x in pairs) / sum(x[1] for x in pairs)


def confidence(a, b=None):
    rng = random.Random(SEED)
    samples = []
    for _ in range(TRIALS):
        idx = [rng.randrange(len(a)) for _ in a]
        den = sum(a[i][1] for i in idx)
        score = sum(a[i][0] for i in idx) / den
        if b is not None:
            score = sum(b[i][0] for i in idx) / den - score
        samples.append(score)
    return [percentile(samples, .025), percentile(samples, .975)]


def metric(rows, field='raw', reference='reference'):
    pairs = [errors(r[reference], r[field] or '') for r in rows]
    return dict(n=len(rows), errors=sum(p[0] for p in pairs),
                words=sum(p[1] for p in pairs), wer=ratio(pairs), ci95=confidence(pairs))


def timing(rows, field):
    vals = [r[field] for r in rows if field in r]
    if not vals:
        return None
    return dict(n=len(vals), median=statistics.median(vals), p95=percentile(vals, .95),
                min=min(vals), max=max(vals))


def paired(base, learned):
    by_id = {r['id']: r for r in learned}
    a = [errors(r['reference'], r['text']) for r in base]
    b = [errors(r['reference'], by_id[r['id']]['text']) for r in base]
    groups = {k: [] for k in ['better', 'worse', 'unchanged']}
    for row, x, y in zip(base, a, b):
        key = 'better' if y[0] < x[0] else 'worse' if y[0] > x[0] else 'unchanged'
        groups[key].append(dict(id=row['id'], reference=row['reference'], before=row['text'],
                               after=by_id[row['id']]['text'], before_errors=x[0], after_errors=y[0],
                               before_raw=row['raw'], after_raw=by_id[row['id']]['raw']))
    return dict(n=len(a), before_wer=ratio(a), after_wer=ratio(b),
                delta=ratio(b)-ratio(a), delta_ci95=confidence(a,b),
                counts={k:len(v) for k,v in groups.items()}, examples=groups)


def correlation(a,b):
    if len(a) < 2:
        return None
    ma, mb = statistics.mean(a), statistics.mean(b)
    num = sum((x-ma)*(y-mb) for x,y in zip(a,b))
    den = math.sqrt(sum((x-ma)**2 for x in a)*sum((y-mb)**2 for y in b))
    return num / den if den else None


def study(path):
    baseline = json.loads((path/'baseline.json').read_text())
    teacher = json.loads((path/'teacher.json').read_text())
    learned = json.loads((path/'learned.json').read_text())
    all_ids = {r['id'] for r in baseline['rows']}
    assert all_ids == {r['id'] for r in learned['rows']}
    teacher_ids = {r['id'] for r in teacher['rows']}
    history_ids = {r['id'] for r in baseline['rows'] if r['split']=='history' and r['status']=='text'}
    assert teacher_ids == all_ids or teacher_ids == history_ids
    out = dict(student=baseline['model'], teacher=teacher['model'], scenarios={},
               timings={}, process_load_ms={k:v.get('load_observations_ms') or [v['load_ms']] for k,v in [('baseline',baseline),('teacher',teacher),('learned',learned)]})
    for name, obj in [('baseline',baseline),('teacher',teacher),('learned',learned)]:
        rows = obj['rows']
        out['timings'][name] = {field:timing(rows,field) for field in ['trim_ms','recognize_ms','rules_ms','pipeline_ms','cpu_ms']}
        rtf = [r['recognize_ms']/1000/r['duration_s'] for r in rows]
        out['timings'][name]['rtf'] = dict(n=len(rtf),median=statistics.median(rtf),p95=percentile(rtf,.95))
        cpu_rows = [r for r in rows if 'cpu_ms' in r]
        out['timings'][name]['cpu_seconds_per_wall_second'] = (sum(r['cpu_ms'] for r in cpu_rows)/sum(r['pipeline_ms'] for r in cpu_rows)) if cpu_rows else None
    for scenario in ['general','recurring']:
        b = [r for r in baseline['rows'] if r['scenario']==scenario]
        t = [r for r in teacher['rows'] if r['scenario']==scenario]
        l = [r for r in learned['rows'] if r['scenario']==scenario]
        hist = [r for r in t if r['split']=='history']
        by_id = {r['id']:r for r in b}
        true_scores = [errors(r['reference'],by_id[r['id']]['raw']) for r in hist]
        true_rates = [e/n for e,n in true_scores]
        displayed = [r['display_wer'] for r in hist]
        learning = json.loads((path/f'{scenario}-learning.json').read_text())
        out['scenarios'][scenario] = dict(student_raw=metric(b),teacher_raw=metric(t),student_raw_matched_teacher=metric([by_id[r['id']] for r in t]),
            teacher_vs_student_matched=paired([dict(by_id[r['id']],text=by_id[r['id']]['raw']) for r in t],[dict(r,text=r['raw']) for r in t]),
            history=paired([r for r in b if r['split']=='history'],l),
            heldout=paired([r for r in b if r['split']=='test'],l),
            teacher_agreement=dict(app_mean_student_vs_teacher_wer=statistics.mean(displayed),
                agreement_score=max(0,min(1,1-statistics.mean(displayed))),
                true_mean_clip_wer=statistics.mean(true_rates),true_corpus_wer=ratio(true_scores),
                per_clip_wer_correlation=correlation(true_rates,displayed),
                standard_student_vs_teacher=metric([dict(reference=r['raw'],raw=by_id[r['id']]['raw']) for r in hist])),
            dictionary=learning['dictionary'],suggestions=learning['suggestions'],examples=learning['examples'],
            review_reports=[r['report'] for r in hist])
    return out


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('study',type=Path)
    args=parser.parse_args()
    # Hand-check the metric against published WER definitions before trusting runs.
    assert errors('the quick brown fox','the quick red fox') == (1,4)
    assert errors('one two','one') == (1,2)
    assert errors('one two','one three four') == (2,2)
    assert errors("Hello, WORLD!",'hello world') == (0,2)
    result=study(args.study)
    (args.study/'summary.json').write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n')
    for scenario,s in result['scenarios'].items():
        print(scenario,json.dumps({k:v for k,v in s.items() if k in ['student_raw','teacher_raw']}))
        print('heldout',json.dumps({k:v for k,v in s['heldout'].items() if k!='examples'}))
    print('timings',json.dumps(result['timings']))


if __name__=='__main__':
    main()
