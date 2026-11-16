#!/usr/bin/env python3
"""History-only mechanism audit, then one final held-out comparison.

Uses the original independent scorer/bootstrap; no reference enters learning.
"""
import argparse
import json
from pathlib import Path
from analyse import metric, paired


def history(directory):
    rows = json.loads((directory / 'history.json').read_text())
    result = {}
    for scenario in ('general', 'recurring'):
        subset = [r for r in rows if r['scenario'] == scenario]
        empty = [r for r in subset if r['phase'] == 'empty']
        result[scenario] = {
            phase: paired(empty, [r for r in subset if r['phase'] == phase])
            for phase in ('single-sighting', 'corroborated')
        }
    return result


def final(original, fixed):
    old = json.loads((original / 'baseline.json').read_text())['rows']
    base = json.loads((fixed / 'baseline.json').read_text())['rows']
    learned = json.loads((fixed / 'learned.json').read_text())['rows']
    assert {r['id'] for r in old} == {r['id'] for r in base} == {r['id'] for r in learned}
    result = {}
    for scenario in ('general', 'recurring'):
        scenario_rows = {}
        for split in ('history', 'test'):
            select = lambda rows: [r for r in rows if r['scenario'] == scenario and r['split'] == split]
            o, b, l = select(old), select(base), select(learned)
            scenario_rows[split] = {
                'original': metric(o, 'text'), 'fixed_empty': metric(b, 'text'),
                'fixed_learned': metric(l, 'text'), 'learning': paired(b, l),
                'all_fixes': paired(o, l),
                'text_changed_by_learning': sum(r['text'] != next(x['text'] for x in l if x['id'] == r['id']) for r in b),
                'raw_changed_by_learning': sum(r['raw'] != next(x['raw'] for x in l if x['id'] == r['id']) for r in b),
            }
        result[scenario] = scenario_rows
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['history', 'final'])
    parser.add_argument('directory', type=Path)
    parser.add_argument('--original', type=Path)
    args = parser.parse_args()
    if args.mode == 'history':
        result = history(args.directory)
    else:
        assert args.original is not None
        result = final(args.original, args.directory)
    dest = args.directory / 'fixes-summary.json'
    dest.write_text(json.dumps(result, indent=2) + '\n')
    for scenario, groups in result.items():
        for name, score in groups.items():
            value = score if args.mode == 'history' else score['learning']
            print(scenario, name, f"{value['before_wer']:.4%} -> {value['after_wer']:.4%}", value['counts'])
    print(dest)


if __name__ == '__main__':
    main()
