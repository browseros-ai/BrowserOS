#!/usr/bin/env python3
"""Prove startup settings in fresh image containers, without repairing startup through the API.

accept.sh owns the containers and root-written fixtures. This probe reads the first settings,
runs real MCP input, inspects persisted traces, and restarts only its recorded server process.
"""

import json
import os
from pathlib import Path
import pwd
import signal
import sqlite3
import sys
import time
import uuid

from accept_client import call_run, connect, host
from trace_values import scan, server_identity

ROOT = Path('/data/browserclaw')
STATE = ROOT / 'claw-sdk'
SAVED = STATE / 'settings.json'
TOKEN = Path('/tmp/settings-host-token')
SOCKET = '/run/neo/host.sock'
URL = 'http://127.0.0.1:9010/mcp'


def browser_identity():
    """Identify the own-container browser by its CDP launch flag and process start time."""
    matches = []
    for path in Path('/proc').glob('[0-9]*/cmdline'):
        try:
            if b'--remote-debugging-port=9110' in path.read_bytes().split(b'\0'):
                started = (path.parent / 'stat').read_text().rsplit(')', 1)[1].split()[19]
                matches.append((int(path.parent.name), started))
        except (OSError, ValueError, IndexError):
            continue
    if len(matches) != 1:
        raise AssertionError(f'expected one browser process, found {len(matches)}')
    return matches[0]


class Proof:
    """Collect saved-file observations and emit a result even when a required step fails."""

    def __init__(self):
        self.checks = []

    def check(self, name, ok, **facts):
        self.checks.append({'name': name, 'ok': bool(ok), **facts})
        if not ok:
            raise AssertionError(name)

    def settings(self, idle=300, retention=3600):
        reply = host(SOCKET, str(TOKEN), '/v1/settings')
        settings = reply.get('body', {}).get('settings', {})
        trace = settings.get('trace', {})
        self.check(f'loaded settings idle={idle} retention={retention}',
                   reply.get('status') == 200 and
                   settings.get('tabs', {}).get('idleCloseSeconds') == idle and
                   trace.get('values') == 'off' and trace.get('screenshots') == 'off' and
                   trace.get('retentionSeconds') == retention)

    def ownership(self):
        neo = pwd.getpwnam('neo')
        folder, saved = STATE.stat(), SAVED.stat()
        self.check('state and settings owned by configured Neo identity, settings mode 0600',
                   (folder.st_uid, folder.st_gid) == (neo.pw_uid, neo.pw_gid) and
                   (saved.st_uid, saved.st_gid) == (neo.pw_uid, neo.pw_gid) and
                   saved.st_mode & 0o777 == 0o600,
                   uid=neo.pw_uid, gid=neo.pw_gid)

    def restart_server(self, authenticated=True):
        """Restart the exact recorded server and prove Chromium's supervisor kept its browser."""
        old, browser = server_identity(), browser_identity()
        self.check('exact server identity rechecked before SIGKILL', server_identity() == old)
        os.kill(old[0], signal.SIGKILL)
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            ready = False
            try:
                new = server_identity()
                if new != old:
                    connect(URL)
                    ready = not authenticated or host(SOCKET, str(TOKEN), '/v1/settings').get('status') == 200
            except (AssertionError, OSError, ValueError, RuntimeError):
                pass
            if ready:
                self.check('server restarted while browser stayed the same',
                           browser_identity() == browser, old=old[0], new=new[0])
                return
            time.sleep(0.25)
        raise AssertionError('server did not restart and become ready')

    def replace_saved(self, body):
        """Publish a host-written fixture atomically before restarting only the server."""
        temporary = SAVED.with_suffix('.host-next')
        temporary.write_text(body)
        neo = pwd.getpwnam('neo')
        os.chown(temporary, neo.pw_uid, neo.pw_gid)
        temporary.chmod(0o600)
        temporary.replace(SAVED)

    def recording_off(self, authenticated=True):
        canary = 'settings-file-canary-' + uuid.uuid4().hex
        code = (f'const canary = {json.dumps(canary)};\n'
                "await page.goto('data:text/html,<title>Startup settings</title><input id=f>');\n"
                "await page.locator('#f').fill(canary);\n"
                "return await page.locator('#f').inputValue() === canary;")
        result = call_run(connect(URL), code, {'taskId': 'settings-file-' + uuid.uuid4().hex})
        run = result.get('meta', {}).get('id')
        self.check('first script filled and read its input',
                   result.get('meta', {}).get('status') == 'ok' and
                   (result.get('value') or {}).get('value') is True, run=run)
        # The database proves tokenless recording behavior too. Wait for the writer's real
        # completion marker rather than assuming the MCP response already reached disk.
        deadline = time.monotonic() + 15
        with sqlite3.connect(f'file:{STATE}/trace.db?mode=ro', uri=True) as database:
            row = None
            while time.monotonic() < deadline:
                row = database.execute('SELECT "values", code, complete FROM runs WHERE id=?', (run,)).fetchone()
                if row and row[2]:
                    break
                time.sleep(0.1)
            self.check('persisted run policy is off and script text is absent',
                       row is not None and row == ('off', None, 1))
            shots = database.execute('SELECT screenshot FROM operations WHERE run_id=?', (run,)).fetchall()
            self.check('no operation stored a screenshot', bool(shots) and all(row[0] != 'stored' for row in shots))
        if authenticated:
            reply = host(SOCKET, str(TOKEN), f'/v1/runs/{run}?wait=10')
            self.check('host record reports values off', reply.get('status') == 200 and
                       reply.get('body', {}).get('values') == 'off')
        files = [path for path in ROOT.rglob('*') if path.is_file()]
        self.check('canary absent from every state file including database, WAL and logs',
                   bool(files) and not scan(files, {'canary': canary})['canary'], files=len(files))

    def run(self, mode):
        self.settings()
        self.ownership()
        self.recording_off()
        if mode == 'saved':
            for body in ['{"trace":', '{"trace":{"values":false,"screenshots":"blurred","retentionSeconds":5},"tabs":"x"}']:
                # The host replaces the saved file before restarting only the server. Keep
                # it readable by Neo so this exercises parsing, not an ownership failure.
                self.replace_saved(body)
                self.restart_server()
                self.settings(1800, 604800)
                self.recording_off()
            self.replace_saved('{"tabs":{"idleCloseSeconds":300},"trace":{"values":"off","screenshots":"off","retentionSeconds":3600}}')
            TOKEN.unlink()
            self.restart_server(authenticated=False)
            self.check('no usable host token leaves no host listener', host(SOCKET, '-', '/v1/settings').get('status') is None)
            self.recording_off(authenticated=False)
        elif mode == 'fixed':
            reply = host(SOCKET, str(TOKEN), '/v1/settings', 'PATCH', '{"tabs":{"idleCloseSeconds":600}}')
            self.check('PATCH applies idle 600', reply.get('status') == 200 and
                       reply.get('body', {}).get('settings', {}).get('tabs', {}).get('idleCloseSeconds') == 600)
            self.restart_server()
            self.settings(600)
            self.ownership()


def linked_identity_case(ready):
    """Judge identity and link ownership independently, even when Neo never becomes ready.

    The fixture wrapper stays alive briefly after a failed entrypoint, so root can still
    inspect both mounted volumes. An unavailable PATCH remains absent from the evidence.
    """
    target = Path('/state-volume/private-state')

    def facts():
        """Missing files retain their own errors without hiding the other volume observations."""
        observed = {'rootIsLink': ROOT.is_symlink()}
        try:
            observed['rootTarget'] = str(ROOT.readlink())
            observed['savedInTarget'] = SAVED.is_file() and SAVED.resolve() == target / 'claw-sdk/settings.json'
        except (OSError, RuntimeError) as error:
            observed['linkError'] = str(error)
        for name, path in [('folder', STATE), ('file', SAVED), ('target', target)]:
            try:
                stat = path.stat()
                observed[name] = {'uid': stat.st_uid, 'gid': stat.st_gid, 'mode': oct(stat.st_mode & 0o777)}
            except OSError as error:
                observed[name] = {'error': str(error)}
        return observed

    def request(method='GET', body=''):
        try:
            return host(SOCKET, str(TOKEN), '/v1/settings', method, body)
        except Exception as error:
            return {'error': str(error)}

    initial, before = request(), facts()
    settings = initial.get('body', {}).get('settings', {})
    trace = settings.get('trace', {})
    loaded = (initial.get('status') == 200 and settings.get('tabs', {}).get('idleCloseSeconds') == 300 and
              trace.get('values') == 'off' and trace.get('screenshots') == 'off' and trace.get('retentionSeconds') == 3600)
    patch = request('PATCH', '{"tabs":{"idleCloseSeconds":600}}') if ready and loaded else None
    patched = bool(patch and patch.get('status') == 200 and
                   patch.get('body', {}).get('settings', {}).get('tabs', {}).get('idleCloseSeconds') == 600)
    after = facts() if patched else None

    def ids_ok(snapshot):
        return bool(snapshot and all(snapshot.get(name, {}).get('uid') == 2307 and
                                     snapshot.get(name, {}).get('gid') == 2307 for name in ('folder', 'file', 'target')) and
                    snapshot.get('file', {}).get('mode') == '0o600')

    def link_ok(snapshot):
        return bool(snapshot and snapshot.get('rootIsLink') and snapshot.get('rootTarget') == str(target) and
                    snapshot.get('savedInTarget') and snapshot.get('target', {}).get('mode') == '0o700')

    results = {}
    for name, check in [('ids', ids_ok), ('link', link_ok)]:
        checks = [{'name': 'Neo ready', 'ok': ready}, {'name': 'initial GET loaded fixed settings', 'ok': loaded},
                  {'name': f'{name} facts before PATCH', 'ok': check(before)},
                  {'name': 'PATCH idle 600 saved', 'ok': patched},
                  {'name': f'{name} facts after PATCH', 'ok': check(after)}]
        problems = [row['name'] for row in checks if not row['ok']]
        detail = ('' if ready else 'not ready; ') + json.dumps({'before': before, 'afterPatch': after}, sort_keys=True)
        results[name] = {'ok': not problems, 'checks': checks, 'problems': problems, 'detail': detail,
                         'ready': ready, 'initialSettings': settings, 'before': before, 'afterPatch': after,
                         'patchStatus': patch.get('status') if patch else None}
    return results


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == 'linked-ids':
        try:
            report = linked_identity_case(len(sys.argv) > 2 and sys.argv[2] == '1')
        except Exception as error:
            failed = {'ok': False, 'checks': [{'name': 'probe completed', 'ok': False}],
                      'problems': [str(error)], 'detail': 'not ready; filesystem facts unavailable',
                      'before': None, 'afterPatch': None}
            report = {'ids': failed, 'link': failed}
        print(json.dumps(report))
        sys.exit(0)
    proof = Proof()
    try:
        proof.run(sys.argv[1])
    except Exception as error:
        proof.checks.append({'name': 'probe completed', 'ok': False, 'error': str(error)})
    problems = [row['name'] for row in proof.checks if not row['ok']]
    print(json.dumps({'ok': not problems, 'checks': proof.checks, 'problems': problems,
                      'detail': f'{len(proof.checks)} startup settings assertions; {len(problems)} failed'}))
