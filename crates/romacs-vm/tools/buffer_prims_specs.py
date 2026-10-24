"""Refresh buffer-area specs from the built production manifest."""
import glob
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
manifest = json.loads(Path(max(glob.glob(os.environ['CARGO_TARGET_DIR'] + '/debug/build/romacs-vm-*/out/primitives.json'), key=os.path.getmtime)).read_text())
probes = {
    'char-before': [
        '(with-temp-buffer (char-before))',
        '(with-temp-buffer (insert "aλ文z") (goto-char 3) (narrow-to-region 2 4) (list (char-before) (char-before 2) (char-before 4) (char-before 5) (point)))',
        '(with-temp-buffer (insert "aλ文z") (list (char-before 1) (char-before 2) (char-before 3) (char-before 4) (char-before 5)))',
        '(with-temp-buffer (insert "aλ文z") (narrow-to-region 3 3) (list (char-before 2) (char-before 3) (char-before 4)))',
        '(with-temp-buffer (insert "aλ文z") (narrow-to-region 2 4) (widen) (char-before 5))',
        '(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 0 127 128 255)) (list (char-before 1) (char-before 2) (char-before 3) (char-before 4) (char-before 5)))',
        '(with-temp-buffer (insert "aλ文z") (let ((m (copy-marker 4))) (list (char-before m) (point))))',
        '(let ((b (get-buffer-create "char-dead")) (m (make-marker))) (with-current-buffer b (insert "λ") (set-marker m 2)) (kill-buffer b) (list (marker-position m) (marker-buffer m) (condition-case e (char-before m) (error e))))',
        '(char-before (make-marker))',
        '(let ((a (get-buffer-create "cb-a")) (b (get-buffer-create "cb-b")) (m (make-marker))) (with-current-buffer a (insert "λx") (set-marker m 2)) (with-current-buffer b (insert "abcdef") (char-before m)))',
        '(let ((a (get-buffer-create "cb-a2")) (b (get-buffer-create "cb-b2")) (m (make-marker))) (with-current-buffer a (insert "abc") (set-marker m 3)) (with-current-buffer b (insert "λx") (char-before m)))',
        '(with-temp-buffer (insert "x") (list (char-before -1) (char-before 0) (char-before #x100000000000000000000000) (char-before #x-100000000000000000000000)))',
        '(char-before 1.0)', '(char-before t)', '(char-before "bad")', '(char-before \'(1))',
        '(list (fboundp \'char-before) (functionp \'char-before))',
        '(with-temp-buffer (insert "λ") (funcall \'char-before))',
    ],
    'buffer-file-name': [
        '(buffer-file-name)',
        '(buffer-file-name nil)',
        '(let ((b (get-buffer-create "bf-empty"))) (buffer-file-name b))',
        '(let ((b (get-buffer-create "bf-live"))) (with-current-buffer b (setq buffer-file-name "λ/文.txt")) (buffer-file-name b))',
        '(let ((b (get-buffer-create "bf-killed"))) (with-current-buffer b (setq buffer-file-name "λ/文.txt")) (kill-buffer b) (buffer-file-name b))',
        '(let ((b (get-buffer-create "bf-killed-empty"))) (kill-buffer b) (buffer-file-name b))',
        '(with-temp-buffer (insert "λ文") (narrow-to-region 2 3) (setq buffer-file-name "narrow") (list (buffer-file-name) (progn (widen) (buffer-file-name))))',
        '(with-temp-buffer (setq buffer-file-name "outer") (let ((buffer-file-name "inner")) (buffer-file-name)))',
        '(with-temp-buffer (let ((name (copy-sequence "λ.txt"))) (setq buffer-file-name name) (eq name (buffer-file-name))))',
        '(let ((a (get-buffer-create "bf-a")) (b (get-buffer-create "bf-b"))) (with-current-buffer a (setq buffer-file-name "A")) (with-current-buffer b (setq buffer-file-name "B") (list (buffer-file-name a) (buffer-file-name b) (buffer-file-name))))',
        '(buffer-file-name "absent")', '(buffer-file-name 7)', '(buffer-file-name t)',
        '(list (fboundp \'buffer-file-name) (functionp \'buffer-file-name))',
        '(with-temp-buffer (setq buffer-file-name "indirect") (funcall \'buffer-file-name (current-buffer)))',
    ],
}
out = ROOT / 'crates/romacs-vm/tools/primh/buffer-area-specs'
out.mkdir(exist_ok=True)
for name, cases in probes.items():
    row = next(row for row in manifest if row['name'] == name)
    spec = {'name': name, 'arity': [row['min'], row['max']], 'effect': row['effect'].lower(), 'gnu_files': [row['gnu'].split(':')[0]], 'gnu_def': row['gnu'], 'rust': row['rust'], 'args': ['projected-buffer-area'] * row['max'], 'probes': cases}
    (out / (name + '.json')).write_text(json.dumps(spec, indent=2) + '\n')
    print(name, len(cases), 'semantic cases')
