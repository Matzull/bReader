#!/usr/bin/env python3
"""Checks that every relative Markdown link and #anchor in the repository resolves.

usage: tools/check-docs.py [repo-root]
"""
import re, os, sys
root = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..")
files = []
for d, ds, fs in os.walk(root):
    ds[:] = [x for x in ds if x not in ('target', '.git', 'data', '.claude', 'node_modules', '.venv', 'pkg', 'pkg-node')]
    for f in fs:
        if f.endswith('.md'):
            files.append(os.path.join(d, f))


def slug(h):
    h = re.sub(r'`', '', h.strip().lower())
    h = re.sub(r'\[([^\]]*)\]\([^)]*\)', r'\1', h)
    h = re.sub(r'[^\w\- ]', '', h, flags=re.UNICODE)
    return h.replace(' ', '-')


anchors = {}


def get_anchors(p):
    if p in anchors:
        return anchors[p]
    s = set()
    seen = {}
    fence = False
    for line in open(p, encoding='utf-8'):
        if line.lstrip().startswith('```'):
            fence = not fence
        if fence:
            continue
        m = re.match(r'#{1,6}\s+(.*)', line)
        if m:
            sl = slug(m.group(1))
            n = seen.get(sl, 0)
            seen[sl] = n + 1
            s.add(sl if n == 0 else f'{sl}-{n}')
    anchors[p] = s
    return s


bad = 0
total = 0
for p in files:
    fence = False
    for i, line in enumerate(open(p, encoding='utf-8'), 1):
        if line.lstrip().startswith('```'):
            fence = not fence
        if fence:
            continue
        line2 = re.sub(r'`[^`]*`', '', line)
        for m in re.finditer(r'\[[^\]]*\]\(([^)\s]+)\)', line2):
            t = m.group(1)
            if re.match(r'^(https?:|mailto:)', t):
                continue
            total += 1
            path, _, frag = t.partition('#')
            tgt = os.path.normpath(os.path.join(os.path.dirname(p), path)) if path else p
            if not os.path.exists(tgt):
                print(f'{p}:{i}: missing {t}')
                bad += 1
                continue
            if frag and tgt.endswith('.md') and frag not in get_anchors(tgt):
                print(f'{p}:{i}: no anchor #{frag} in {tgt}')
                bad += 1
print(f'{len(files)} files, {total} links, {bad} broken')
sys.exit(1 if bad else 0)
