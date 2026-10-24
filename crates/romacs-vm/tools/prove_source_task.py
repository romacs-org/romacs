#!/usr/bin/env python3
"""Real local productivity proof: file query and read-only notmuch query.
The caller sends source text and waits for each eval-stream reply before EOF.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    crate = Path(__file__).resolve().parents[1]
    process = subprocess.Popen([args.binary, '--eval-stream', '--gc-floor', '1'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    stdin, stdout = process.stdin, process.stdout
    assert stdin is not None and stdout is not None
    def evaluate(source):
        stdin.write(json.dumps({'source':source}) + '\n')
        stdin.flush()
        reply = json.loads(stdout.readline())
        assert 'error' not in reply, reply
        return reply['value']
    rows = []
    with tempfile.TemporaryDirectory(prefix='vm2-source-task-', dir=Path.home() / '.hermes/cache/scratch') as folder:
        path = Path(folder) / 'actions.txt'
        path.write_text('TODO reply to review\nDONE publish\nTODO pay invoice\n')
        source = f'''(with-temp-buffer
          (insert-file-contents {json.dumps(str(path))})
          (goto-char (point-min))
          (let ((count 0))
            (while (search-forward "TODO" nil t) (setq count (1+ count))) count))'''
        result = evaluate(source)
        oracle = subprocess.check_output(['/usr/bin/emacs', '-Q', '--batch', '--eval', f'(prin1 {source})'], text=True)
        assert result == oracle == '2', (result, oracle)
        rows.append({'task':'read file into buffer and count action items', 'source':source, 'vm2':result, 'gnu':oracle})
    inbox_source = (crate / 'examples/agent-inbox.el').read_text()
    result = evaluate(inbox_source)
    direct = subprocess.check_output(['notmuch', 'count', '--output=messages', 'tag:inbox and tag:unread'], text=True).strip()
    expected = json.dumps(f'{direct} unread inbox messages')
    oracle = subprocess.check_output(['/usr/bin/emacs', '-Q', '--batch', '-l', str(crate / 'examples/agent-inbox.el'), '--eval', '(prin1 (agent-unread-inbox))'], text=True)
    assert result == expected == oracle, (result, expected, oracle)
    # Prove persistent source-defined functions remain agent-callable.
    assert evaluate('(agent-unread-inbox)') == result
    rows.append({'task':'read-only real notmuch unread-inbox summary', 'source_file':'examples/agent-inbox.el', 'vm2':result, 'direct_notmuch':direct, 'gnu':oracle, 'persistent_second_call':True})
    stdin.close()
    assert process.wait(timeout=10) == 0
    report = {'transport':'existing eval-stream; source requests; reply before EOF', 'tasks':rows}
    Path(args.output).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))

if __name__ == '__main__':
    main()
