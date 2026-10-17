#!/usr/bin/env python3
"""Rehearse a reviewed rank plan; execute only its SHA-bound, revision-checked edits.

No discovery, due-date edits, retries or automatic rollback. A partial/unknown
result requires inspection of the receipt before further action.
"""
import argparse
import hashlib
import json
import re
from decimal import Decimal
from pathlib import Path
import subprocess


def run(args):
    raw = Path(args.plan).read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if args.execute and args.plan_sha256 != digest:
        raise ValueError('execution requires the exact reviewed --plan-sha256')
    document = json.loads(raw)
    if not isinstance(document, dict) or document.get('quipu') != args.quipu or document.get('graph') != args.graph:
        raise ValueError('plan route differs from reviewed target')
    plan = document.get('rows')
    if Path(args.receipt).exists():
        raise ValueError('receipt exists; inspect it before another run')
    if not isinstance(plan, list) or not 0 < len(plan) <= 100:
        raise ValueError('plan must contain 1..100 explicit rows')
    ids = [r['id'] for r in plan]
    if len(set(ids)) != len(ids):
        raise ValueError('duplicate plan item')
    command = [args.sd, '--quipu', args.quipu, '--graph', args.graph]
    def call(verb):
        p = subprocess.run(command + verb + ['--json'], capture_output=True, text=True, timeout=120)
        if p.returncode:
            raise RuntimeError('sd refused or outcome UNKNOWN; inspect receipt, do not retry')
        return json.loads(p.stdout)
    observed = call(['show', *ids])
    if not isinstance(observed, list) or len(observed) != len(plan):
        raise ValueError('incomplete target coverage')
    current = {r['id']: r for r in observed}
    for row in plan:
        actual = current[row['id']]
        if actual.get('status') in ('closed', 'tombstone'):
            raise ValueError('plan includes terminal work; obtain a current disposition')
        for field in ('priority', 'revision', 'rank', 'due_at'):
            if field not in row['expected'] or actual.get(field) != row['expected'][field]:
                raise ValueError('plan precondition differs: ' + row['id'] + '/' + field)
        rank = row['rank']
        if rank is not None:
            if (not isinstance(rank, str) or not re.fullmatch(r'-?[0-9]+(?:\.[0-9]{1,9})?', rank)
                    or abs(Decimal(rank)) > Decimal('9000000000000000000')):
                raise ValueError('rank must be a bounded exact decimal or null')
            canonical = format(Decimal(rank), 'f')
            if '.' in canonical:
                canonical = canonical.rstrip('0').rstrip('.')
            if Decimal(rank) == 0:
                canonical = '0'
            if canonical != rank:
                raise ValueError('plan rank must be canonical decimal text')
        if type(row['expected']['revision']) is not int or type(row['expected']['priority']) is not int:
            raise ValueError('plan revision/priority must be integers')
    receipt = {'plan_sha256': digest, 'execute': args.execute, 'targets': ids, 'applied': [], 'rollback_plan': []}
    output = Path(args.receipt)
    def save():
        output.write_text(json.dumps(receipt, indent=2) + '\n')
    save()
    try:
        if args.execute:
            for row in plan:
                receipt['pending_id'] = row['id']
                save()
                result = call(['update', row['id'], '--rank', row['rank'] or '',
                               '--if-revision', str(row['expected']['revision'])])
                if not isinstance(result, list) or len(result) != 1:
                    raise ValueError('update outcome UNKNOWN')
                after = result[0]
                if (after['id'] != row['id'] or after['due_at'] != row['expected']['due_at']
                        or after['rank'] != row['rank'] or after['priority'] != row['expected']['priority']):
                    raise ValueError('post-write coverage UNKNOWN')
                receipt['applied'].append(after['id'])
                receipt['rollback_plan'].append({'id': after['id'], 'rank': row['expected']['rank'],
                    'expected': {k: after[k] for k in ('priority', 'revision', 'rank', 'due_at')}})
                receipt.pop('pending_id', None)
                save()
        receipt['state'] = 'APPLIED' if args.execute else 'REHEARSED_NO_WRITES'
    except Exception:
        receipt['state'] = 'STOPPED_INSPECT_PENDING_AND_APPLIED'
        save()
        raise
    save()
    print(json.dumps({'state': receipt['state'], 'targets': len(ids), 'plan_sha256': digest}))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', required=True)
    p.add_argument('--receipt', required=True)
    p.add_argument('--sd', default='sd')
    p.add_argument('--quipu', required=True)
    p.add_argument('--graph', required=True)
    p.add_argument('--execute', action='store_true')
    p.add_argument('--plan-sha256')
    args = p.parse_args()
    run(args)


if __name__ == '__main__':
    main()
