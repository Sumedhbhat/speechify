#!/usr/bin/env python3
"""Audit formatter exact matches, fallback causes, and cleanup-only learning."""
import argparse
import json
from pathlib import Path

from analyse import paired, timing


def formatting(path):
    rows=json.loads(path.read_text())
    assert len(rows) in [20,52] and len({r['id'] for r in rows})==len(rows)
    calls=[call for r in rows for call in r['calls']]
    accepted=[r for r in rows if r['rewritten']]
    return dict(n=len(rows),correct=sum(r['correct'] for r in rows),
        by_source={s:dict(n=sum(r['source']==s for r in rows),correct=sum(r['source']==s and r['correct'] for r in rows)) for s in ['existing','new']},
        accepted_empty=sum(not r['text'] and not r['fallback'] and bool(r['calls']) for r in rows),
        accepted_rewrites=len(accepted),accepted_rewrites_correct=sum(r['correct'] for r in accepted),
        fallbacks=sum(bool(r['fallback']) for r in rows),
        vet_rejections=sum(bool(r['fallback']) and r['fallback'].startswith("The language model's answer") for r in rows),
        request_errors=sum('Err' in call['answer'] for call in calls),
        elapsed_ms=timing(rows,'elapsed_ms'),call_ms=timing(calls,'ms'),
        failures=[r for r in rows if not r['correct']])


def main():
    parser=argparse.ArgumentParser();parser.add_argument('root',type=Path);args=parser.parse_args()
    out={}
    for name in ['rules','gemma','gemma-quality']:
        path=args.root/f'format-{name}.json'
        if path.exists():out[name]=formatting(path)
    path=args.root/'parakeet-gemma/cleanup.json'
    if path.exists():
        rows=json.loads(path.read_text())
        out['cleanup']={}
        for scenario in ['general','recurring']:
            selected=[r for r in rows if r['scenario']==scenario]
            before=[r for r in selected if r['phase']=='test-empty']
            after=[r for r in selected if r['phase']=='test-learned']
            assert len(before)==len(after)==(30 if scenario=='general' else 12)
            learning=json.loads((path.parent/f'{scenario}-learning.json').read_text())
            out['cleanup'][scenario]=dict(heldout=paired(before,after),
                learning=learning,
                phases={phase:dict(n=len(group),rewritten=(sum(r['rewritten'] for r in group) if all('rewritten' in r for r in group) else None),
                    fallback=(sum(bool(r['fallback']) for r in group) if all('fallback' in r for r in group) else None),elapsed_ms=timing(group,'elapsed_ms'),
                    calls=len([c for r in group for c in r['calls']]),
                    request_errors=sum('Err' in c['answer'] for r in group for c in r['calls']))
                    for phase in ['history-seed','review','test-empty','test-learned']
                    if (group:=[r for r in selected if r['phase']==phase])})
        all_calls=[c for r in rows for c in r['calls']]
        out['cleanup']['call_ms']=timing(all_calls,'ms')
    (args.root/'cleanup-summary.json').write_text(json.dumps(out,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps({k:{f:v for f,v in a.items() if f not in ['failures','cleanup']} for k,a in out.items() if k!='cleanup'},indent=2))
    if 'cleanup' in out:
        for s in ['general','recurring']:
            print(s,json.dumps({k:v for k,v in out['cleanup'][s]['heldout'].items() if k!='examples'}))


if __name__=='__main__':
    main()
