#!/usr/bin/env python3
"""Image-level file-policy proof through MCP and the authenticated host socket.

Run as root inside an expendable acceptance container so fixtures and settings-byte checks
remain host work. Only paths cross into run scripts; host credentials never do. --baseline
reproduces the three historical attacks and restores the saved settings bytes afterwards.
"""

import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import http.server
import json
import os
from pathlib import Path
import tempfile
import threading
import uuid

from accept_client import FilesPage, STATEMENT, call_run, connect, host


HTML = """<title>file policy</title><input type=file id=f><input type=file id=directory webkitdirectory>
<div id=zone style='width:100px;height:100px'>Drop</div><p id=seen></p>
<script>
document.getElementById('zone').addEventListener('dragover', event => event.preventDefault());
document.getElementById('zone').addEventListener('drop', async event => {
  event.preventDefault();
  const file = event.dataTransfer.files[0];
  document.getElementById('seen').textContent = file.name + ':' + await file.text();
});
</script>"""
PREPARE = "await page.goto('data:text/html,' + encodeURIComponent(" + json.dumps(HTML) + "));\n"
FILE_FACTS = """async input => {
  const file = input.files[0];
  return { name: file.name, size: file.size };
}"""


def process_places(token_file):
    """The entrypoint adds directories to the browser/server environment after Docker starts."""
    values = dict(os.environ)
    for proc in Path('/proc').glob('[0-9]*'):
        try:
            command = (proc / 'cmdline').read_bytes().split(b'\0')[0]
            if not command.endswith((b'/claw-sdk', b'/neo-server', b'/browseros-claw-sdk')):
                continue
            values.update(item.decode().split('=', 1) for item in
                          (proc / 'environ').read_bytes().split(b'\0') if b'=' in item)
            break
        except (OSError, UnicodeError):
            continue
    data = values.get('NEO_DATA_DIR', '/data')
    return {
        'state': Path(values.get('BROWSERCLAW_DIR', data + '/browserclaw')) / 'claw-sdk',
        'profile': Path(values.get('CLAW_PROFILE_DIR', data + '/profile')),
        'downloads': Path(values.get('CLAW_DOWNLOADS_DIR', data + '/downloads')),
        'token': Path(token_file),
        'captcha': [Path(values[key]) for key in
                    ('BROWSEROS_CAPSOLVER_API_KEY_FILE', 'BROWSEROS_2CAPTCHA_API_KEY_FILE')
                    if values.get(key)],
    }


@contextlib.contextmanager
def files_server(handler=FilesPage):
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_address[1]}'
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


class Proof:
    """Keep public run/trace observations, never arbitrary script results or credential bytes."""

    def __init__(self, url, socket_path, token_file):
        self.client = connect(url)
        self.socket_path, self.token_file = socket_path, token_file
        self.task = 'accept-files-' + uuid.uuid4().hex[:12]
        self.checks = []

    def api(self, method, body=None):
        return host(self.socket_path, self.token_file, '/v1/settings', method,
                    '' if body is None else json.dumps(body))

    def run(self, code):
        return call_run(self.client, code, {'taskId': self.task})

    def check(self, name, ok, **facts):
        self.checks.append({'name': name, 'ok': bool(ok), **facts})

    def value(self, result):
        return (result.get('value') or {}).get('value')

    def patch(self, dirs):
        reply = self.api('PATCH', {'files': {'readDirs': dirs}})
        self.check('PATCH files.readDirs', reply.get('status') == 200 and
                   reply.get('body', {}).get('settings', {}).get('files', {}).get('readDirs') == dirs)

    def allowed(self, name, code):
        got = self.run(code)
        self.check(name, got['meta'].get('status') == 'ok' and self.value(got) is True,
                   run=got['meta'].get('id'), status=got['meta'].get('status'))
        return got

    def refused(self, name, got, path, own=False):
        error = got['meta'].get('error') or (got.get('value') or {}).get('error') or {}
        run_id = got['meta'].get('id')
        record = host(self.socket_path, self.token_file, f'/v1/runs/{run_id}?wait=10')
        operations = record.get('body', {}).get('operations', [])
        operation = next((op for op in operations if op['id'] == error.get('operationId')), {})
        trace_error = operation.get('error') or {}
        messages = [error.get('message', ''), trace_error.get('message', '')]
        rule = all(('own' in message.lower() if own else 'files.readDirs' in message)
                   and (len(str(path)) == 1 or str(path) not in message) for message in messages)
        ok = (got['meta'].get('status') == 'error' and
              (error.get('owner'), error.get('code'), error.get('delivery')) ==
              ('script', 'invalid_input', 'not_sent') and
              (trace_error.get('owner'), trace_error.get('code'), operation.get('delivery')) ==
              ('script', 'invalid_input', 'not_sent') and operation.get('sentAt') is None and rule)
        self.check(name, ok, run=run_id, owner=error.get('owner'), code=error.get('code'),
                   delivery=error.get('delivery'), traceDelivery=operation.get('delivery'),
                   ruleMessage=rule)
        return got

    def denied(self, name, operation, path, own=False, prepare=PREPARE):
        return self.refused(name, self.run(prepare + operation + '; return true;'), path, own)

    def result(self):
        problems = [row['name'] for row in self.checks if not row['ok']]
        return {'ok': not problems, 'checks': self.checks, 'problems': problems,
                'detail': f'{len(self.checks)} file-policy assertions; {len(problems)} failed'}


def baseline(proof, places):
    """Reproduce reads without returning file bytes, and undo the one intentional overwrite."""
    original_settings = proof.api('GET').get('body', {}).get('settings', {})
    proof.api('PATCH', {'tabs': {'idleCloseSeconds': 301}})
    settings = places['state'] / 'settings.json'
    original_bytes = settings.read_bytes()
    original_stat = settings.stat()
    try:
        trace = places['state'] / 'trace.db'
        uploaded = proof.run(PREPARE +
            f"await page.locator('#f').setInputFiles({json.dumps(str(trace))});\n" +
            f"return await page.locator('#f').evaluate({FILE_FACTS});")
        facts = proof.value(uploaded) or {}
        proof.check('baseline trace.db upload reaches page',
                    uploaded['meta'].get('status') == 'ok' and facts.get('size', 0) > 0,
                    run=uploaded['meta'].get('id'), size=facts.get('size'))
        tagged = proof.run(PREPARE +
            f"const tag = await page.addScriptTag({{path:{json.dumps(str(places['token']))}}});\n" +
            "return await tag.evaluate(el => ({loaded:el.textContent.length>0,"
            "marker:window.__uploadFolderTokenWasRead===true}));")
        facts = proof.value(tagged) or {}
        proof.check('baseline token path reaches script tag',
                    tagged['meta'].get('status') == 'ok' and facts.get('loaded') is True,
                    run=tagged['meta'].get('id'), marker=facts.get('marker'))
        with files_server() as origin:
            saved = proof.run(f"await page.goto({json.dumps(origin + '/')});\n" +
                "const [d] = await Promise.all([page.waitForEvent('download'), page.locator('#get').click()]);\n" +
                f"await d.saveAs({json.dumps(str(settings))}); return true;")
        proof.check('baseline download overwrites settings.json',
                    saved['meta'].get('status') == 'ok' and settings.read_bytes() == STATEMENT,
                    run=saved['meta'].get('id'), changed=settings.read_bytes() != original_bytes)
    finally:
        settings.write_bytes(original_bytes)
        os.chmod(settings, original_stat.st_mode)
        os.chown(settings, original_stat.st_uid, original_stat.st_gid)
        proof.api('PUT', original_settings)
    return proof.result()


def upload(path, selector='#f'):
    return f'await page.locator({json.dumps(selector)}).setInputFiles({json.dumps(str(path))})'


def upload_matches(path, name, text='ordinary'):
    return (PREPARE + upload(path) + ';\n' +
            "return await page.locator('#f').evaluate(async input => { const f=input.files[0]; " +
            f'return f.name==={json.dumps(name)} && await f.text()==={json.dumps(text)}; }});')


def six_reads(proof, inside, outside):
    """Each operation sees the same ordinary fixture on either side of the allowed-root boundary."""
    for root, allowed in ((outside, False), (inside, True)):
        text, script, style = (root / name for name in ('readable.txt', 'script.js', 'style.css'))
        cases = [
            ('setInputFiles', text, upload(text),
             "return await page.locator('#f').evaluate(async el => await el.files[0].text()==='ordinary');"),
            ('drop', text, f"await page.locator('#zone').drop({{files:{json.dumps(str(text))}}})",
             "await expect(page.locator('#seen')).toHaveText('readable.txt:ordinary'); return true;"),
            ('addScriptTag', script, f'await page.addScriptTag({{path:{json.dumps(str(script))}}})',
             "return await page.evaluate(() => window.__filePolicy==='ordinary');"),
            ('addStyleTag', style, f'await page.addStyleTag({{path:{json.dumps(str(style))}}})',
             "return await page.locator('body').evaluate(el => getComputedStyle(el).color==='rgb(12, 34, 56)');"),
        ]
        for receiver in ('page', 'context'):
            cases.append((receiver + '.addInitScript', script,
                          f'await {receiver}.addInitScript({{path:{json.dumps(str(script))}}})',
                          "await page.goto('data:text/html,<p>init</p>'); "
                          "return await page.evaluate(() => window.__filePolicy==='ordinary');"))
        for name, path, operation, result in cases:
            if allowed:
                proof.allowed(name + ' inside allowed folder', PREPARE + operation + ';\n' + result)
            else:
                proof.denied(name + ' outside allowed folder', operation, path)


def live_change(proof, origin_root):
    """The page's marker pauses one run while the host updates settings outside model JS."""
    arrived, release = threading.Event(), threading.Event()

    class Barrier(FilesPage):
        def do_GET(self):
            if self.path not in ('/marker', '/continue'):
                return super().do_GET()
            if self.path == '/marker':
                arrived.set()
            elif not release.wait(timeout=30):
                self.send_error(504)
                return
            self.send_response(200)
            self.send_header('Content-Length', '0')
            self.end_headers()

    proof.patch([str(origin_root)])
    with files_server(Barrier) as origin, ThreadPoolExecutor(max_workers=1) as pool:
        code = (f'await page.goto({json.dumps(origin + "/")});\n' +
                "let before; try { await page.locator('#f').setInputFiles('/etc/hostname'); } "
                "catch(e) { before=[e.owner,e.code,e.delivery]; }\n" +
                "await page.evaluate(async () => { await fetch('/marker'); await fetch('/continue'); });\n" +
                "await page.locator('#f').setInputFiles('/etc/hostname');\n" +
                "return {before,after:await page.locator('#f').evaluate(el=>el.files[0].name==='hostname')};")
        future = pool.submit(proof.run, code)
        try:
            marked = arrived.wait(timeout=20)
            proof.check('running script reached host PATCH barrier', marked)
            if marked:
                proof.patch(['/etc', str(origin_root)])
        finally:
            release.set()
        got = future.result(timeout=80)
        value = proof.value(got) or {}
        proof.check('files.readDirs changes inside same run',
                    got['meta'].get('status') == 'ok' and
                    value.get('before') == ['script', 'invalid_input', 'not_sent'] and value.get('after') is True,
                    run=got['meta'].get('id'))


def protected_writes(proof, places, outside, origin, snapshots):
    """Compare host bytes and missing parent folders as well as API refusal and delivery."""
    state, profile = places['state'], places['profile']
    marker = outside.name
    targets = [state / 'settings.json', state / (marker + '-file.png'),
               state / (marker + '-new') / 'nested' / 'file.png',
               profile / (marker + '-new') / 'nested' / 'file.png',
               places['token'], *places['captcha']]
    # Profile snapshots contain only host-created fixtures, never mutable browser session bytes.
    targets.extend(path for path in snapshots if profile in path.parents)
    link = outside / 'protected-state'
    link.symlink_to(state, target_is_directory=True)
    targets.append(link / (marker + '-link') / 'nested' / 'file.png')
    targets.append(outside / 'missing' / '..' / link.name / (marker + '-transition') / 'file.png')
    missing = [state / (marker + '-new'), profile / (marker + '-new'),
               state / (marker + '-link'), state / (marker + '-transition'), outside / 'missing']
    download_prepare = (f'await page.goto({json.dumps(origin + "/")});\n' +
                        "const [d] = await Promise.all([page.waitForEvent('download'), page.locator('#get').click()]);\n")
    writers = [
        ('download.saveAs', download_prepare, 'await d.saveAs(PATH)'),
        ('page.screenshot', PREPARE, "await page.screenshot({path:PATH,type:'png'})"),
        ('locator.screenshot', PREPARE, "await page.locator('#zone').screenshot({path:PATH,type:'png'})"),
        ('element.screenshot', PREPARE + "const el=await page.locator('#zone').elementHandle();\n",
         "await el.screenshot({path:PATH,type:'png'})"),
    ]
    for name, prepare, operation in writers:
        for index, target in enumerate(targets):
            got = proof.run(prepare + operation.replace('PATH', json.dumps(str(target))) + '; return true;')
            unchanged = all(path.read_bytes() == raw for path, raw in snapshots.items())
            proof.check(f'{name} protected bytes unchanged #{index}', unchanged)
            # A broken guard must not destroy the host credential needed to inspect its trace.
            for path, raw in snapshots.items():
                if path.read_bytes() != raw:
                    path.write_bytes(raw)
            proof.refused(f'{name} protected destination #{index}', got, target, own=True)
            proof.check(f'{name} refused before mkdir #{index}', all(not path.exists() for path in missing))
        working = outside / (name.replace('.', '-') + '.png')
        got = proof.run(prepare + operation.replace('PATH', json.dumps(str(working))) + '; return true;')
        raw = working.read_bytes() if working.exists() else b''
        proof.check(name + ' /tmp still writable', got['meta'].get('status') == 'ok' and
                    (raw == STATEMENT if name == 'download.saveAs' else raw.startswith(b'\x89PNG\r\n\x1a\n')),
                    run=got['meta'].get('id'))

        # Parent creation follows the original spelling, even when its final target escapes
        # through '..'. Protect intermediate directories as well as the eventual file.
        temporary_settings = state / 'settings.json.tmp'
        existed = temporary_settings.exists()
        escaped_file = outside / ('parent-escape-' + name.replace('.', '-') + '.png')
        escape = (str(temporary_settings) + '/' + '../' * (len(temporary_settings.parts) - 1)
                  + str(escaped_file).lstrip('/'))
        proof.check(name + ' settings temporary path starts absent', not existed)
        proof.denied(name + ' refuses protected intermediate mkdir',
                     operation.replace('PATH', json.dumps(escape)), escape, own=True, prepare=prepare)
        proof.check(name + ' creates no settings temporary directory', not temporary_settings.exists())
        if not existed and temporary_settings.is_dir():
            temporary_settings.rmdir()

    # Exercise a real save rather than a no-op update, then restore exactly the prior list.
    proof.patch(['/', '/tmp'])
    proof.patch(['/'])
    proof.check('settings remain writable after intermediate-directory refusals',
                (state / 'settings.json').read_bytes() == snapshots[state / 'settings.json'])

    pdf = outside / 'pdf-path.pdf'
    got = proof.run(PREPARE + f'await page.pdf({{path:{json.dumps(str(pdf))}}}); return true;')
    error = got['meta'].get('error') or {}
    if got['meta'].get('status') == 'ok':
        proof.check('PDF path supported and writes /tmp', pdf.exists() and pdf.read_bytes().startswith(b'%PDF-'))
        for index, target in enumerate(targets):
            proof.denied(f'page.pdf protected destination #{index}',
                         f'await page.pdf({{path:{json.dumps(str(target))}}})', target, own=True)
    else:
        proof.check('PDF path remains explicitly unsupported', error.get('owner') == 'script' and
                    'omit path' in error.get('message', '') and not pdf.exists(),
                    run=got['meta'].get('id'), code=error.get('code'))
    proof.check('protected writes left no new parent directories', all(not path.exists() for path in missing))


def policy(proof, places):
    initial = proof.api('GET')
    settings = initial.get('body', {}).get('settings', {})
    defaults = initial.get('body', {}).get('defaults', {})
    downloads = places['downloads']
    default_dirs = ['/data/downloads']
    proof.check('image defaults are downloads-folder-only', initial.get('status') == 200 and
                defaults.get('files', {}).get('readDirs') == default_dirs and
                settings.get('files', {}).get('readDirs') == default_dirs)
    if not proof.checks[-1]['ok']:
        return proof.result()
    with tempfile.TemporaryDirectory(prefix='neo-accept-files-', dir=downloads) as inside_name, \
         tempfile.TemporaryDirectory(prefix='neo-accept-files-', dir='/tmp') as outside_name, \
         tempfile.TemporaryDirectory(prefix='downloads-old-', dir=downloads.parent) as sibling_name, \
         files_server() as origin:
        inside, outside = Path(inside_name), Path(outside_name)
        os.chmod(inside, 0o755)
        os.chmod(outside, 0o777)
        sibling = Path(sibling_name)
        os.chmod(sibling, 0o755)
        (sibling / 'readable.txt').write_text('ordinary')
        for root in (inside, outside):
            (root / 'readable.txt').write_text('ordinary')
            (root / 'script.js').write_text("window.__filePolicy='ordinary';")
            (root / 'style.css').write_text('body { color: rgb(12, 34, 56) }')
        profile = places['profile']
        profile_fixtures = [
            ('profile synthetic Session_ fixture', profile / 'Default' / 'Sessions' / 'Session_neo_accept'),
            ('profile synthetic Tabs_ fixture', profile / 'Default' / 'Sessions' / 'Tabs_neo_accept'),
            ('profile synthetic deep fixture', profile / 'Default' / 'Storage' / 'neo-accept' / 'deep' / 'form-state.dat'),
        ]
        created_profile_files, created_profile_dirs = [], []
        try:
            proof.denied('default refuses /etc/hostname', upload('/etc/hostname'), '/etc/hostname')
            six_reads(proof, inside, outside)
            escaped = inside / 'escape.txt'
            escaped.symlink_to(outside / 'readable.txt')
            proof.denied('allowed symlink cannot escape', upload(escaped), escaped)
            proof.denied('same-prefix sibling folder is outside the list',
                         upload(sibling / 'readable.txt'), sibling / 'readable.txt')
            walked = inside / '..' / '..' / sibling.name / 'readable.txt'
            proof.denied('dot-dot cannot leave allowed folder', upload(walked), walked)
            alias = inside / 'alias.txt'
            alias.symlink_to(inside / 'readable.txt')
            proof.allowed('inside symlink keeps upload basename', upload_matches(alias, 'alias.txt'))
            incoming = outside / 'incoming-alias.txt'
            incoming.symlink_to(inside / 'readable.txt')
            proof.allowed('outside symlink into allowed folder keeps basename',
                          upload_matches(incoming, 'incoming-alias.txt'))
            walked_out = str(inside / '..' / '..' / 'profile' / 'Local State')
            proof.denied('dot-dot cannot reach protected profile', upload(walked_out), walked_out, own=True)

            proof.patch(['/etc', str(downloads)])
            proof.allowed('PATCH /etc enables ordinary hostname upload', PREPARE +
                          upload('/etc/hostname') + "; return await page.locator('#f').evaluate(el=>el.files[0].name==='hostname');")
            live_change(proof, downloads)
            proof.patch([str(inside / 'does-not-exist')])
            proof.denied('missing allowed folder matches nothing',
                         upload(inside / 'readable.txt'), inside / 'readable.txt')
            proof.patch([])
            proof.denied('empty readDirs refuses a downloaded-folder path', upload(inside / 'readable.txt'),
                         inside / 'readable.txt')
            proof.allowed('payload and clearing need no path allowance', PREPARE +
                          "await page.locator('#f').setInputFiles({name:'owner.txt',mimeType:'text/plain',buffer:Buffer.from('owner bytes')});\n"
                          "const ok=await page.locator('#f').evaluate(async el=>await el.files[0].text()==='owner bytes');\n"
                          "await page.locator('#f').setInputFiles([]); return ok && await page.locator('#f').evaluate(el=>el.files.length===0);")
            proof.patch([str(downloads)])
            proof.allowed('file chooser shares allowed upload path', PREPARE +
                          "const [chooser]=await Promise.all([page.waitForEvent('filechooser'),page.locator('#f').click()]);\n" +
                          f'await chooser.setFiles({json.dumps(str(inside / "readable.txt"))});\n' +
                          "return await page.locator('#f').evaluate(async el=>await el.files[0].text()==='ordinary');")

            proof.patch(['/'])
            # Exclusive creation and ownership tracking preserve any preexisting browser files.
            for name, path in profile_fixtures:
                missing_parents, parent = [], path.parent
                while not parent.exists():
                    missing_parents.append(parent)
                    parent = parent.parent
                for parent in reversed(missing_parents):
                    parent.mkdir()
                    created_profile_dirs.append(parent)
                with path.open('xb') as fixture:
                    created_profile_files.append(path)
                    fixture.write(('Harmless synthetic acceptance marker: ' + name + '\n').encode())
            protected = [('trace', places['state'] / 'trace.db'),
                         ('settings', places['state'] / 'settings.json'),
                         ('profile', places['profile'] / 'Local State'),
                         ('token', places['token'])]
            protected.extend(profile_fixtures)
            existing_keys = [path for path in places['captcha'] if path.is_file()]
            proof.check('configured CAPTCHA file is included', bool(existing_keys))
            protected.extend((f'captcha-{index}', path) for index, path in enumerate(existing_keys))
            for index, path in enumerate(places['captcha']):
                if path.is_symlink() and not path.exists():
                    target = Path(os.path.realpath(path))
                    proof.denied(f'dangling credential alias remains protected #{index}',
                                 upload(path), path, own=True)
                    proof.denied(f'missing credential target remains protected #{index}',
                                 upload(target), target, own=True)
                    proof.denied(f'dangling credential target cannot be created #{index}',
                                 f"await page.screenshot({{path:{json.dumps(str(target))},type:'png'}})",
                                 target, own=True)
                    proof.check(f'dangling credential target stays absent #{index}', not target.exists())
                    proof.allowed(f'dangling credential permits unrelated allowed read #{index}',
                                  upload_matches(inside / 'readable.txt', 'readable.txt'))
            for name, path in protected:
                proof.check(name + ' fixture exists', path.is_file())
                for kind, candidate in [('direct', path),
                                        ('dot-dot', path.parent / '..' / path.parent.name / path.name)]:
                    operation = (f'await page.addScriptTag({{path:{json.dumps(str(candidate))}}})'
                                 if name == 'token' else upload(candidate))
                    proof.denied(name + ' ' + kind + ' with readDirs root', operation, candidate, own=True)
                link = outside / (name + '-link')
                link.symlink_to(path)
                proof.denied(name + ' symlink with readDirs root', upload(link), link, own=True)
            ancestors = {'/', str(places['state'].parent), str(places['profile'].parent),
                         str(places['token'].parent)}
            for path in sorted(ancestors):
                proof.denied('directory upload refuses protected ancestor ' + path,
                             upload(path, '#directory'), path, own=True)
            snapshots = {path: path.read_bytes() for name, path in protected if name not in ('trace', 'profile')}
            protected_writes(proof, places, outside, origin, snapshots)
            proof.check('synthetic session and nested profile fixture bytes unchanged',
                        all(path.read_bytes() == snapshots[path] for name, path in profile_fixtures))
            proof.check('settings bytes unchanged by all refused writes',
                        (places['state'] / 'settings.json').read_bytes() == snapshots[places['state'] / 'settings.json'])
        finally:
            for path in created_profile_files:
                path.unlink(missing_ok=True)
            for parent in reversed(created_profile_dirs):
                # The browser may have added files to a directory after the host created it.
                with contextlib.suppress(OSError):
                    parent.rmdir()
            proof.api('PUT', settings)
    return proof.result()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mcp_url')
    parser.add_argument('socket')
    parser.add_argument('token_file')
    parser.add_argument('--baseline', action='store_true')
    args = parser.parse_args()
    proof = Proof(args.mcp_url, args.socket, args.token_file)
    places = process_places(args.token_file)
    out = baseline(proof, places) if args.baseline else policy(proof, places)
    print(json.dumps(out))
    raise SystemExit(0 if out['ok'] else 1)


if __name__ == '__main__':
    main()
