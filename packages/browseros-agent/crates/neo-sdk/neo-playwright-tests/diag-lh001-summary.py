"""DIAGNOSTIC (diag/ss054-own-window, never lands): per group, passes per test and Neo's input timings.

Reads each group's results folder under the given root: Playwright's results.json for outcomes and
durations, and every worker's trace.db for the time between Neo sending an input and the action
ending (`sent_at` -> `ended_at`), which is where LH-001 loses ~1 s per mouse action on Linux.
"""
import glob
import json
import os
import sqlite3
import statistics
import sys

IDS = {
    'page/page-add-locator-handler.spec.ts › should work': 'LH-001',
    'page/page-add-locator-handler.spec.ts › should work with toHaveScreenshot': 'LH-008',
    'page/elementhandle-screenshot.spec.ts › element screenshot › should wait for visible': 'EH-085',
    'page/elementhandle-screenshot.spec.ts › element screenshot › should wait for element to stop moving': 'EH-088',
    'page/page-screenshot.spec.ts › page screenshot animations › should not capture css animations in shadow DOM': 'SS-054',
    'page/page-screenshot.spec.ts › page screenshot › should capture canvas changes': 'SS-024',
    'page/page-screenshot.spec.ts › page screenshot animations › should not capture infinite css animation': 'SS-052',
}


def walk(suite, path, out):
    title = suite.get('title', '')
    here = path + ([title] if title else [])
    for spec in suite.get('specs', []):
        key = ' › '.join(here + [spec['title']])
        for test in spec.get('tests', []):
            for result in test.get('results', []):
                out.setdefault(key, []).append((result.get('status'), result.get('duration', 0)))
    for child in suite.get('suites', []):
        walk(child, here, out)


def spread(values):
    if not values:
        return 'none'
    values = sorted(values)
    p90 = values[min(len(values) - 1, int(len(values) * 0.9))]
    return f'n={len(values)} median={statistics.median(values):.0f} p90={p90} max={values[-1]} ms'


def group_report(group, folder):
    print(f'\n### Group {group}\n')
    for run_json in sorted(glob.glob(os.path.join(folder, '*', 'run.json')))[:1]:
        print(f"ownWindowPages: `{json.load(open(run_json)).get('ownWindowPages')}`\n")
    launches = sorted(glob.glob(os.path.join(folder, '*', 'worker-*', 'launch.json')))
    if launches:
        flags = json.load(open(launches[0]))['flags']
        print(f'Launch flags (worker 0 of {len(launches)}): `{" ".join(flags[:-1])}`\n')
    outcomes = {}
    for results in glob.glob(os.path.join(folder, '*', 'results.json')):
        report = json.load(open(results))
        for suite in report.get('suites', []):
            walk(suite, [], outcomes)
    print('| Test | Passed | Runs | Statuses | Duration median / max |')
    print('|---|--:|--:|---|---|')
    for key, case in IDS.items():
        runs = outcomes.get(key, [])
        statuses = {}
        for status, _ in runs:
            statuses[status] = statuses.get(status, 0) + 1
        durations = [d for _, d in runs]
        timing = f'{statistics.median(durations) / 1000:.1f} s / {max(durations) / 1000:.1f} s' if durations else '-'
        print(f'| {case} | {statuses.get("passed", 0)} | {len(runs)} | {statuses} | {timing} |')
    missing = [k for k in outcomes if k not in IDS]
    if missing:
        print(f'\nOther tests in the run: {missing[:5]}')
    posts = {'hover #aside': [], 'click #close': [], 'click #target': []}
    shots = {}
    for db in glob.glob(os.path.join(folder, '*', 'worker-*', 'claw-state', 'claw-sdk', 'trace.db')):
        con = sqlite3.connect(db)
        rows = con.execute(
            "select name, json_extract(args, '$.target.display'), started_at, sent_at, ended_at, outcome "
            'from operations where ended_at is not null'
        ).fetchall()
        con.close()
        for name, target, started, sent, ended, outcome in rows:
            if 'screenshot' in name.lower():
                # A capture is a read: it has no send, so its whole time counts.
                shots.setdefault(name, []).append(ended - started if outcome == 'ok' else None)
                continue
            if sent is None:
                continue
            post = ended - sent
            if outcome == 'ok' and name == 'locator.hover' and target == "locator('#aside')":
                posts['hover #aside'].append(post)
            elif outcome == 'ok' and name == 'locator.click' and target == "locator('#close')":
                posts['click #close'].append(post)
            elif outcome == 'ok' and name == 'locator.click' and target == "locator('#target')":
                posts['click #target'].append(post)
    print('\nNeo trace, time after the input is sent (LH-001\'s actions; ok outcomes only):\n')
    for kind, values in posts.items():
        print(f'- {kind}: {spread(values)}')
    print('\nScreenshot operations, whole time:\n')
    for name, values in sorted(shots.items()):
        ok = [v for v in values if v is not None]
        print(f'- {name}: {spread(ok)}; not ok: {len(values) - len(ok)}')


def main(root):
    print('## SS-054 placement diagnostic: B = background tabs (before), W = own windows (after)')
    for folder in sorted(glob.glob(os.path.join(root, '*'))):
        if os.path.isdir(folder):
            group_report(os.path.basename(folder), folder)


if __name__ == '__main__':
    main(sys.argv[1])
