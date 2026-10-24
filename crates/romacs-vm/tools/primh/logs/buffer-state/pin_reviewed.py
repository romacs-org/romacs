#!/usr/bin/env python3
"""Explicit reviewed pins; never infer semantic exclusions from a run."""
import json
from pathlib import Path
root=Path(__file__).resolve().parents[2]
pins={
 'overlayp': {'overlayp/0040':'PRINTER'},
 'buffer-live-p': {'buffer-live-p/0042':'PRINTER'},
 'buffer-name': {**{f'buffer-name/{i:04}':'PRINTER' for i in (18,27,45)},'buffer-name/0034':'VALUE','buffer-name/0037':'VALUE','buffer-name/0038':'DATA'},
 'buffer-modified-p': {**{f'buffer-modified-p/{i:04}':'PRINTER' for i in (18,27,47)},'buffer-modified-p/0039':'DATA','buffer-modified-p/0040':'MISSING'},
}
for name,pinned in pins.items():
 p=root/'evidence'/f'{name}.json'; e=json.loads(p.read_text()); e['pinned']=pinned; p.write_text(json.dumps(e,ensure_ascii=False,indent=1)+'\n')
