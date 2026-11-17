// Disposable Neo only. NEO_CDP and VIEWER_BASE must point to the container started for this test.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { lookup } from 'node:dns/promises';
const { chromium, devices } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const neoBase = process.env.NEO_CDP;
const viewerBase = process.env.VIEWER_BASE;
assert.ok(neoBase && viewerBase, 'Set NEO_CDP and VIEWER_BASE to a disposable Neo container');
const cdpBase = new URL(neoBase);
cdpBase.hostname = (await lookup(cdpBase.hostname, { family: 4 })).address;
const info = await (await fetch(new URL('/json/version', cdpBase))).json();
const endpoint = new URL(info.webSocketDebuggerUrl); endpoint.host = cdpBase.host;
const neo = await chromium.connectOverCDP(endpoint.href);
const phone = await chromium.launch();
const remote = await neo.contexts()[0].newPage();
try {
  await remote.goto('data:text/html;charset=utf-8,' + encodeURIComponent('<meta charset="utf-8"><textarea id="field" autofocus style="width:90%;height:300px;font-size:20px"></textarea>'));
  await remote.bringToFront();
  const context = await phone.newContext({ ...devices['iPhone 13'], permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  const html = readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8');
  await context.route('https://clip.test/**', route => route.fulfill({ contentType: 'text/html', body: html }));
  await context.routeWebSocket('wss://clip.test/slim', socket => {
    // Forward unchanged records to the real slimviewer through its TCP/WebSocket proxy.
    const upstream = new WebSocket(viewerBase.replace(/^http/, 'ws') + '/stream');
    upstream.binaryType = 'arraybuffer';
    upstream.onmessage = event => socket.send(Buffer.from(event.data));
    socket.onMessage(message => upstream.send(message));
    socket.onClose(() => upstream.close());
    upstream.onclose = () => socket.close();
  });
  await page.goto('https://clip.test/');
  await page.locator('#cover').waitFor({ state: 'hidden', timeout: 20000 });
  const samples = ['héllo 日本 👍', 'café déjà vu', 'a\nb\tc', 'e\u0301 Ελληνικά 中文 👩🏽‍💻', 'héllo 日本 👍'.repeat(30)];
  for (let repeat = 1; repeat <= 3; repeat++) {
    for (const text of samples) {
      await remote.locator('#field').fill(''); await remote.locator('#field').focus();
      await page.evaluate(text => navigator.clipboard.writeText(text), text);
      await page.locator('#paste-btn').tap();
      await remote.waitForFunction(text => document.getElementById('field').value === text, text, { timeout: 5000 });
      const actual = await remote.locator('#field').inputValue();
      assert.equal(actual, text);
      console.log(JSON.stringify({ repeat, bytes: Buffer.byteLength(text), text, actual, pass: true }));
    }
  }
  await remote.locator('#field').fill(''); await remote.locator('#field').focus();
  const accepted = await page.evaluate(() => {
    window.apiInputs = 0;
    window.slimviewer.addEventListener('input', () => window.apiInputs++);
    return [window.slimviewer.type('héllo 日本 👍'), window.slimviewer.key('Backspace'),
      window.slimviewer.key('Enter'), window.slimviewer.type('done')];
  });
  assert.deepEqual(accepted, [true, true, true, true]);
  await remote.waitForFunction(() => document.getElementById('field').value === 'héllo 日本 \ndone');
  assert.equal(await page.evaluate(() => window.apiInputs), 4);
  console.log(JSON.stringify({ api: 'type/key/input', actual: await remote.locator('#field').inputValue(), pass: true }));
  console.log(JSON.stringify({ browser: await neo.version(), frames: await page.evaluate(() => window.__stats.frames), pass: true }));
} finally {
  await remote.close(); await phone.close(); await neo.close();
}
