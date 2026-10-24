"""Generate bounded buffer-local primh specs from the production manifest.

The semantic probes also back the direct value tests. Ordinary generated
argument domains are intentionally excluded: raw buffer-local-variables returns
an inventory of native slots outside this lane's implemented substrate. This
suite compares projections of that inventory, not a false complete inventory.
"""
import glob
import json
import os
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[3]
manifest_path = max(glob.glob(os.environ['CARGO_TARGET_DIR'] + '/debug/build/romacs-vm-*/out/primitives.json'), key=os.path.getmtime)
manifest = {row['name']: row for row in json.loads(Path(manifest_path).read_text())}
tests = (ROOT / 'crates/romacs-vm/tests/buffer_local_substrate.rs').read_text()
probes = [json.loads('"' + text + '"') for text in re.findall(r'check\("((?:\\.|[^"\\])*)",', tests)]
probes += [
    "(with-temp-buffer (setq buffer-file-name \"visit\" buffer-read-only t buffer-file-format '(plain)) (kill-all-local-variables t) (list buffer-file-name buffer-read-only buffer-file-format))",
    "(with-temp-buffer (setq buffer-file-name \"visit\") (kill-all-local-variables 'reset) buffer-file-name)",
    "(with-temp-buffer (setq fill-column 33) (put 'fill-column 'permanent-local t) (kill-all-local-variables) (list fill-column (local-variable-p 'fill-column)))",
    "(with-temp-buffer (setq bl-h '(a b t)) (make-local-variable 'bl-h) (put 'bl-h 'permanent-local 'permanent-local-hook) (put 'b 'permanent-local-hook t) (kill-all-local-variables) bl-h)",
    "(with-temp-buffer (setq bl-void 2) (make-local-variable 'bl-void) (makunbound 'bl-void) (list (not (null (memq 'bl-void (buffer-local-variables)))) (local-variable-p 'bl-void)))",
    "(with-temp-buffer (setq bl-x 1) (make-local-variable 'bl-x) (let ((bl-x 2)) (kill-local-variable 'bl-x)) (list bl-x (local-variable-p 'bl-x)))",
    "(with-temp-buffer (setq fill-column 22) (let ((fill-column 33)) (kill-local-variable 'fill-column)) (list fill-column (local-variable-p 'fill-column)))",
    "(condition-case e (buffer-local-value 'x nil) (error e))",
    "(condition-case e (local-variable-p 'x 3) (error e))",
    "(condition-case e (buffer-local-variables 3) (error e))",
    "(list (kill-local-variable nil) (kill-local-variable t) (local-variable-p nil) (local-variable-p t))",
    "(with-temp-buffer (let ((a (make-symbol \"x\"))) (set a 4) (make-local-variable a) (set a 7) (kill-local-variable a) (symbol-value a)))",
    "(with-temp-buffer (setq bl-x 3) (defvaralias 'bl-alias 'bl-x) (make-local-variable 'bl-x) (setq bl-x 4) (kill-local-variable 'bl-alias) (list bl-x (local-variable-p 'bl-alias)))",
    "(with-temp-buffer (list (cdr (assq 'major-mode (buffer-local-variables))) (equal (cdr (assq 'mode-name (buffer-local-variables))) \"Fundamental\") (assq 'fill-column (buffer-local-variables))))",
]
out = ROOT / 'crates/romacs-vm/tools/primh/buffer-local-specs'
out.mkdir(exist_ok=True)
probes += [row['source'] for row in json.loads((ROOT / 'crates/romacs-vm/evidence/overlay-missing.json').read_text())['cases']]
for name in ['kill-all-local-variables', 'kill-local-variable', 'local-variable-p', 'buffer-local-value', 'buffer-local-variables', 'delete-all-overlays', 'overlay-buffer', 'previous-overlay-change']:
    row = manifest[name]
    chosen = [p for p in probes if name in p]
    call = {
        'kill-all-local-variables': "(with-temp-buffer (funcall 'kill-all-local-variables))",
        'kill-local-variable': "(with-temp-buffer (funcall 'kill-local-variable 'fill-column))",
        'local-variable-p': "(with-temp-buffer (funcall 'local-variable-p 'fill-column))",
        'buffer-local-value': "(with-temp-buffer (funcall 'buffer-local-value 'fill-column (current-buffer)))",
        'buffer-local-variables': "(with-temp-buffer (consp (funcall 'buffer-local-variables)))",
        'delete-all-overlays': "(with-temp-buffer (funcall 'delete-all-overlays))",
        'overlay-buffer': "(with-temp-buffer (eq (current-buffer) (funcall 'overlay-buffer (make-overlay 1 1))))",
        'previous-overlay-change': "(with-temp-buffer (funcall 'previous-overlay-change 1))",
    }[name]
    chosen += [f"(list (fboundp '{name}) (functionp '{name}))", call]
    spec = {'name': name, 'arity': [row['min'], row['max']], 'effect': row['effect'].lower(), 'gnu_files': [row['gnu'].split(':')[0]], 'gnu_def': row['gnu'], 'rust': row['rust'], 'args': ['projected-buffer-locals'] * row['max'], 'probes': chosen}
    (out / (name + '.json')).write_text(json.dumps(spec, indent=2) + '\n')
    print(name, len(chosen), 'semantic probes; generated arity boundaries added by primh')
