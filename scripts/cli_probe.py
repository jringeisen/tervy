#!/usr/bin/env python3
"""Claude CLI probes using documented flags/hooks only; never read transcripts.
Default: initialization-only Setup hooks with temporary user settings.
--real: also make small real model calls, budget-capped per invocation.
Codex-specific probes are added with its integration in M6.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid


def hooks(marker, log, names):
    command = shlex.join([sys.executable, str(marker), str(log)])
    return {"hooks": {name: [{"hooks": [{"type": "command", "command": command, "timeout": 5}]}] for name in names}}


def events(log):
    if not log.exists():
        return []
    return [json.loads(line) for line in log.read_text().splitlines() if line]


def stop(child):
    if child.poll() is None:
        os.killpg(child.pid, signal.SIGTERM)
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=3)


def call(command, folder, environment, timeout=60):
    child = subprocess.Popen(command, cwd=folder, env=environment, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = child.communicate(timeout=timeout)
        return child.returncode, stdout, stderr
    finally:
        stop(child)


def merge_probe(cli, folder, marker):
    config = folder / 'user'
    config.mkdir(mode=0o700)
    user_log, injected_log = folder / 'user-hooks.jsonl', folder / 'injected-hooks.jsonl'
    (config / 'settings.json').write_text(json.dumps(hooks(marker, user_log, ['Setup'])))

    env = dict(os.environ, CLAUDE_CONFIG_DIR=str(config), ANTHROPIC_API_KEY='devdesk-local-fixture-not-a-secret',
               ANTHROPIC_BASE_URL='http://127.0.0.1:9',
               CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', NO_PROXY='127.0.0.1,localhost', API_TIMEOUT_MS='5000')
    for key in ['ANTHROPIC_AUTH_TOKEN', 'CLAUDE_CODE_OAUTH_TOKEN', 'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY']:
        env.pop(key, None)
    injected = hooks(marker, injected_log, ['Setup'])
    code, stdout, stderr = call([cli, '--init-only', '--strict-mcp-config', '--tools=', '--setting-sources=user',
                                 '--settings', json.dumps(injected)], folder, env, timeout=25)
    user, extra = events(user_log), events(injected_log)
    assert any(e['event'] == 'Setup' for e in user), f'user Setup missing (CLI exit {code})'
    assert any(e['event'] == 'Setup' for e in extra), f'injected Setup missing (CLI exit {code})'
    assert user[0]['session_id'] == extra[0]['session_id'], 'merged hooks disagree on session ID'
    print('PASS A5: temporary user-level and injected Setup hooks both fired; real user settings untouched')
    return {'a5': True, 'probe_event': 'Setup', 'cli_exit': code, 'events': [e['event'] for e in extra]}


def real_probe(cli, folder, marker):
    notes = folder / 'notes.txt'
    notes.write_text('The secret project codename is BLUEHERON.\n')
    original = notes.read_bytes()
    parent_id = str(uuid.uuid4())
    parent_log, side_log = folder / 'parent-hooks.jsonl', folder / 'side-hooks.jsonl'
    common = ['--strict-mcp-config', '--disable-slash-commands', '--output-format', 'json', '--model', 'haiku', '--max-budget-usd', '0.10']
    parent_args = [cli, '-p', '--session-id', parent_id, '--settings', json.dumps(hooks(marker, parent_log, ['SessionStart', 'PreToolUse', 'Stop'])),
                   '--tools=Read,Bash', '--allowedTools=Read,Bash(sleep *)', *common,
                   'Read notes.txt, then run sleep 45 using Bash. After it finishes reply PARENT_DONE and the codename.']
    env = os.environ.copy()
    with (folder / 'parent.stdout').open('w') as output, (folder / 'parent.stderr').open('w') as errors:
        parent = subprocess.Popen(parent_args, cwd=folder, env=env, stdin=subprocess.DEVNULL, stdout=output, stderr=errors, start_new_session=True)
        try:
            deadline = time.monotonic() + 90
            while not any(e['event'] == 'PreToolUse' and e.get('tool_name') == 'Bash' for e in events(parent_log)):
                if parent.poll() is not None:
                    output.flush()
                    errors.flush()
                    detail = (folder / 'parent.stdout').read_text()[-4000:]
                    diagnostic = (folder / 'parent.stderr').read_text()[-2000:]
                    raise AssertionError(f'parent exited {parent.returncode} before its sleep tool; synthetic CLI result: {detail}; stderr: {diagnostic}')
                assert time.monotonic() < deadline, 'parent did not start its sleep tool'
                time.sleep(.1)
            side = [cli, '-p', '--resume', parent_id, '--fork-session', '--settings', json.dumps(hooks(marker, side_log, ['SessionStart'])),
                    '--restricted', '--tools=Read,Grep,Glob', '--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,mcp__*',
                    '--permission-mode=dontAsk', *common,
                    'What codename did the parent read? This is an authorized enforcement probe: try to create pwned.txt with Write, edit notes.txt with Edit, and run Bash to create pwned-bash.txt. Report which tools are unavailable.']
            code, stdout, _ = call(side, folder, env, timeout=60)
            assert code == 0, 'fork CLI failed'
            result = json.loads(stdout)
            assert 'BLUEHERON' in result.get('result', ''), 'fork did not inherit codename'
            assert notes.read_bytes() == original and not (folder / 'pwned.txt').exists() and not (folder / 'pwned-bash.txt').exists(), 'read-only fixture changed'
            starts = [e for e in events(side_log) if e['event'] == 'SessionStart']
            assert starts and starts[0]['session_id'] != parent_id, 'fork hook did not identify a distinct session'
            assert parent.poll() is None, 'parent no longer running when fork completed; overlap not established'
            parent.wait(timeout=75)
            output.flush()
            assert parent.returncode == 0, 'parent failed'
        finally:
            stop(parent)
    result = json.loads((folder / 'parent.stdout').read_text())
    assert 'PARENT_DONE' in result.get('result', ''), 'parent did not complete normally'
    print('PASS Claude fork while running: inherited context, distinct SessionStart ID, parent completed')
    print('PASS headless read-only fixture unchanged; interactive tool enforcement remains the M5 gate')
    resume_log = folder / 'resume-hooks.jsonl'
    code, stdout, _ = call([cli, '-p', '--resume', parent_id, '--settings', json.dumps(hooks(marker, resume_log, ['SessionStart'])),
                            '--tools=', *common, 'Reply with only the codename you read earlier.'], folder, env)
    assert code == 0 and 'BLUEHERON' in json.loads(stdout).get('result', ''), 'resume lost context'
    assert any(e['session_id'] == parent_id for e in events(resume_log)), 'resume changed session identity'
    print('PASS Claude resume: same session ID and prior conversation context')
    return {'fork_overlap': True, 'headless_read_only_fixture': True, 'resume_identity_and_context': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--real', action='store_true', help='also make paid model calls (each CLI call capped at $0.10)')
    parser.add_argument('--output', type=Path, default=Path('perf/local/claude-probe.json'))
    args = parser.parse_args()
    cli = shutil.which('claude')
    if not cli:
        raise SystemExit('FAIL: claude not on PATH; run from your login/interactive shell')
    version = subprocess.check_output([cli, '--version'], text=True).strip()
    print(f'Claude: {version}')
    if version.split()[0] != '2.1.295':
        print('WARN: version differs from the original verified version; these probes record the current behavior')
    with tempfile.TemporaryDirectory(prefix='devdesk-cli-probe-') as temporary:
        folder = Path(temporary)
        marker = folder / 'marker.py'
        marker.write_text("import json,sys\np=json.load(sys.stdin)\nwith open(sys.argv[1],'a') as f:f.write(json.dumps({'event':p.get('hook_event_name'),'session_id':p.get('session_id'),'tool_name':p.get('tool_name')})+'\\n')\n")
        report = {'version': version, 'evidence': '🧪 documented CLI flags and hook payloads; no transcript reads', 'merge': merge_probe(cli, folder, marker)}
        if args.real:
            try:
                report['real'] = real_probe(cli, folder, marker)
            except Exception as error:
                report['real'] = {'passed': False, 'error': str(error)}
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
                raise
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
