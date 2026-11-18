// Run with Playwright 1.63: PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node --test tests/clipboard-browser.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

const { chromium, webkit, devices } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const html = readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8');
const adapter = readFileSync(new URL('../web/ui.js', import.meta.url), 'utf8');
const text = 'héllo 日本 👍';
async function eventually(check) {
  const deadline = Date.now() + 5000;
  while (!check() && Date.now() < deadline) await delay(20);
  assert.ok(check(), 'expected clipboard text on the wire');
}
function record(type, payload) {
  const bytes = Buffer.alloc(payload.length + 5);
  bytes.writeUInt32BE(payload.length + 1); bytes[4] = type; bytes.set(payload, 5);
  return bytes;
}
const picture = Buffer.alloc(16);
picture.writeUInt32BE(1); picture[4] = 1;
picture.writeUInt16BE(390, 5); picture.writeUInt16BE(844, 7);
picture.set([0, 0, 1, 0x67, 0x42, 0xc0, 0x1f], 9);

async function fixture(browser, run, { insecure = false, native = false, viewOnly = false } = {}) {
  const context = await browser.newContext({ ...devices['iPhone 13'], serviceWorkers: 'block' });
  const page = await context.newPage(), pastes = [], dialogs = [], errors = [], sockets = [];
  page.on('dialog', async dialog => { dialogs.push(dialog.type()); await dialog.dismiss(); });
  page.on('pageerror', error => errors.push(error.message));
  await page.clock.install();
  await context.route('**/*', route => route.fulfill(new URL(route.request().url()).pathname === '/ui.js'
    ? { contentType: 'text/javascript', body: adapter } : { contentType: 'text/html', body: html }));
  await context.routeWebSocket('**/slim', socket => {
    sockets.push(socket);
    socket.onMessage(bytes => {
      const data = Buffer.from(bytes);
      if (data[4] === 0x22) pastes.push(new TextDecoder('utf-8', { fatal: true }).decode(data.subarray(5)));
    });
    socket.send(record(1, [1, Number(viewOnly) | 2]));
    socket.send(record(2, picture));
  });
  await context.addInitScript(({ native }) => {
    // Only the video source is faked. The real page owns transport, clicks, focus and clipboard policy.
    window.VideoDecoder = class {
      static async isConfigSupported() { return { supported: true }; }
      constructor({ output }) { this.output = output; this.state = 'unconfigured'; }
      configure() { this.state = 'configured'; }
      decode(chunk) {
        const frame = document.createElement('canvas'); frame.width = 390; frame.height = 844;
        Object.assign(frame, { timestamp: chunk.timestamp, displayWidth: 390, displayHeight: 844, close() {} });
        this.output(frame);
      }
      close() { this.state = 'closed'; }
    };
    window.EncodedVideoChunk = class { constructor(value) { Object.assign(this, value); } };
    window.probe = { reads: [], focus: [], pending: [], outcomes: [], permission: 'prompt' };
    let trustedClick = false;
    const listen = EventTarget.prototype.addEventListener;
    EventTarget.prototype.addEventListener = function(type, handler, options) {
      if (type !== 'click' || typeof handler !== 'function') return listen.call(this, type, handler, options);
      return listen.call(this, type, function(event) {
        trustedClick = event.isTrusted;
        try { return handler.call(this, event); } finally { trustedClick = false; }
      }, options);
    };
    for (const method of ['focus', 'blur']) {
      const original = HTMLElement.prototype[method];
      HTMLElement.prototype[method] = function(...args) {
        if (this.id === 'noVNC_keyboardinput') probe.focus.push({ method, trustedClick });
        return original.apply(this, args);
      };
    }
    if (!native) {
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
        readText() {
          probe.reads.push({ trustedClick, active: navigator.userActivation.isActive });
          return new Promise((resolve, reject) => probe.pending.push({ resolve, reject }));
        },
      } });
      Object.defineProperty(navigator, 'permissions', { configurable: true, value: {
        async query() {
          if (probe.permission === 'unsupported') throw new TypeError('Unknown permission');
          return { state: probe.permission };
        },
      } });
    }
  }, { native });
  try {
    await page.goto(`${insecure ? 'http' : 'https'}://viewer.test/`);
    await page.waitForFunction(() => window.__stats?.frames === 1);
    await page.clock.runFor(2100);
    await page.locator('#cover').waitFor({ state: 'hidden' });
    const tap = () => page.locator('#paste-btn').tap();
    const settle = (index, value, reject = false) => page.evaluate(({ index, value, reject }) => {
      const read = probe.pending[index];
      reject ? read.reject(new DOMException(value, 'NotAllowedError')) : read.resolve(value);
    }, { index, value, reject });
    const state = async expected => {
      await page.waitForFunction(expected => document.getElementById('controls').dataset.paste === expected, expected, { timeout: 2000 });
      assert.equal(await page.locator('#paste-btn').textContent(), { idle: 'Paste', active: 'Paste', cancelled: 'Tap again', blocked: 'Paste blocked' }[expected]);
    };
    await run({ page, context, tap, settle, state, pastes, sockets, dialogs });
    assert.deepEqual(dialogs, [], 'paste must never open a prompt or panel');
    assert.deepEqual(errors, [], 'page must not throw');
  } finally { await context.close(); }
}

test('tests that an unfocused iframe pastes the real clipboard from the original tap', async t => {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const context = await browser.newContext({ ...devices['iPhone 13'], permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage(), pastes = [], errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await context.route('https://viewer.test/**', route => route.fulfill({ contentType: 'text/html', body:
    new URL(route.request().url()).pathname === '/parent'
      ? '<button id="paste">Paste into viewer</button><iframe src="/viewer" style="width:390px;height:844px"></iframe><script>paste.onclick=()=>{window.result=document.querySelector("iframe").contentWindow.slimviewer.paste()}</script>'
      : html }));
  await context.routeWebSocket('**/slim', socket => {
    socket.onMessage(bytes => {
      const data = Buffer.from(bytes);
      if (data[4] === 0x22) pastes.push(new TextDecoder().decode(data.subarray(5)));
    });
    socket.send(record(1, [1, 2]));
  });
  await context.addInitScript(() => {
    window.VideoDecoder = class { static async isConfigSupported() { return { supported: true }; } };
  });
  await page.goto('https://viewer.test/parent');
  const frame = page.frames().find(frame => frame.url().endsWith('/viewer'));
  await frame.waitForFunction(() => window.slimviewer?.connected);
  await page.evaluate(value => navigator.clipboard.writeText(value), text);
  await page.locator('#paste').focus();
  assert.equal(await frame.evaluate(() => document.hasFocus()), false);
  await page.locator('#paste').tap();
  assert.equal(await page.evaluate(() => window.result), 'sent');
  await eventually(() => pastes.length === 1);
  assert.deepEqual(pastes, [text]);
  assert.equal(await frame.evaluate(() => document.hasFocus()), true);
  assert.deepEqual(errors, []);
});

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  test(`tests that ${name} phone paste obeys clipboard and focus rules`, async t => {
    const browser = await engine.launch();
    t.after(() => browser.close());
    await t.test('tests that a trusted tap reads immediately and sends Unicode once', () => fixture(browser, async f => {
      await f.tap();
      assert.deepEqual(await f.page.evaluate(() => probe.reads), [{ trustedClick: true, active: true }]);
      await f.state('active'); await f.settle(0, text);
      await f.page.waitForFunction(() => probe.pending.length === 1);
      await f.page.clock.runFor(1600);
      await f.state('idle'); assert.deepEqual(f.pastes, [text]);
    }));
    await t.test('tests that a cancelled read invites another tap without a prompt', () => fixture(browser, async f => {
      await f.page.evaluate(() => { probe.permission = 'unsupported'; });
      await f.tap(); await f.settle(0, 'Dismissed bubble', true);
      await f.state('cancelled'); assert.deepEqual(f.pastes, []);
      await f.page.clock.runFor(3100); await f.state('idle');
    }));
    await t.test('tests that denied clipboard permission is blocked', () => fixture(browser, async f => {
      await f.page.evaluate(() => { probe.permission = 'denied'; });
      await f.tap(); await f.settle(0, 'Blocked in site settings', true);
      await f.state('blocked'); assert.deepEqual(f.pastes, []);
    }));
    await t.test('tests that a missing clipboard API is blocked', () => fixture(browser, async f => {
      await f.page.evaluate(() => { Object.defineProperty(navigator, 'clipboard', { value: undefined }); });
      await f.tap(); await f.state('blocked'); assert.deepEqual(f.pastes, []);
    }));
    await t.test('tests that an insecure origin is blocked even if a clipboard shim exists', () => fixture(browser, async f => {
      assert.equal(await f.page.evaluate(() => isSecureContext), false);
      await f.tap(); await f.state('blocked');
      assert.deepEqual(await f.page.evaluate(() => probe.reads), []); assert.deepEqual(f.pastes, []);
    }, { insecure: true }));
    await t.test('tests that a double tap discards the cancelled first read and retries the latest', () => fixture(browser, async f => {
      await f.tap(); await f.tap(); await f.settle(0, 'Second tap cancelled first read', true);
      await f.state('active'); await f.settle(1, 'Bubble dismissed', true); await f.state('cancelled');
      await f.tap(); await f.settle(2, text); await f.page.clock.runFor(1600);
      await f.state('idle'); assert.deepEqual(f.pastes, [text]);
    }));
    await t.test('tests that an earlier success still pastes without replacing a newer refusal', () => fixture(browser, async f => {
      await f.tap(); await f.tap(); await f.settle(1, 'Dismissed', true); await f.state('cancelled');
      await f.settle(0, text); await f.state('cancelled'); assert.deepEqual(f.pastes, [text]);
    }));
    await t.test('tests that stale refusal cannot replace a newer success', () => fixture(browser, async f => {
      await f.tap(); await f.tap(); await f.settle(1, text); await f.settle(0, 'Dismissed', true);
      await f.state('idle'); await f.page.clock.runFor(1600); await f.state('idle'); assert.deepEqual(f.pastes, [text]);
    }));
    await t.test('tests that reconnect invalidates a pending read and clears its label', () => fixture(browser, async f => {
      await f.tap(); await f.sockets[0].close();
      await f.page.waitForFunction(() => !window.slimviewer.connected);
      await f.page.clock.runFor(2400);
      await f.page.waitForFunction(() => window.slimviewer.connected);
      await f.settle(0, 'old session'); await f.state('idle'); assert.deepEqual(f.pastes, []);
    }));
    await t.test('tests that keyboard focus and blur stay in trusted clicks while paste preserves focus', () => fixture(browser, async f => {
      const keyboard = f.page.locator('#kbd-btn');
      await keyboard.tap(); assert.equal(await f.page.evaluate(() => document.activeElement.id), 'noVNC_keyboardinput');
      await f.tap(); await f.settle(0, text);
      assert.equal(await f.page.evaluate(() => document.activeElement.id), 'noVNC_keyboardinput');
      await keyboard.tap(); assert.notEqual(await f.page.evaluate(() => document.activeElement.id), 'noVNC_keyboardinput');
      assert.deepEqual(await f.page.evaluate(() => probe.focus), [
        { method: 'focus', trustedClick: true }, { method: 'blur', trustedClick: true },
      ]);
    }));
    await t.test('tests that the pill adapter returns each attempt outcome', () => fixture(browser, async f => {
      await f.page.evaluate(async () => {
        const { pasteFromPill } = await import('/ui.js');
        const button = document.createElement('button'); button.id = 'pill'; button.textContent = 'Pill paste';
        Object.assign(button.style, { position: 'fixed', top: '10px', zIndex: 5 }); document.body.append(button);
        button.addEventListener('click', () => pasteFromPill().then(outcome => probe.outcomes.push(outcome)));
      });
      await f.page.locator('#pill').tap(); await f.page.locator('#pill').tap();
      await f.settle(1, text); await f.settle(0, 'Dismissed', true);
      assert.deepEqual(await f.page.evaluate(() => [...probe.outcomes].sort()), ['cancelled', 'sent']);
      assert.deepEqual(f.pastes, [text]);
      await f.page.locator('#pill').tap(); await f.settle(2, 'Dismissed', true);
      assert.deepEqual(await f.page.evaluate(() => [...probe.outcomes].sort()), ['cancelled', 'cancelled', 'sent']);
      assert.ok((await f.page.evaluate(() => probe.reads)).every(read => read.trustedClick));
    }));
    await t.test('tests that an empty clipboard sends nothing', () => fixture(browser, async f => {
      await f.tap(); await f.settle(0, ''); await f.page.clock.runFor(1600); await f.state('idle');
      assert.deepEqual(f.pastes, []);
    }));
    await t.test('tests that spectators cannot read or send the clipboard', () => fixture(browser, async f => {
      assert.equal(await f.page.evaluate(() => window.slimviewer.paste()), 'blocked');
      assert.deepEqual(await f.page.evaluate(() => probe.reads), []); assert.deepEqual(f.pastes, []);
    }, { viewOnly: true }));
    await t.test('tests that the real browser clipboard reaches the wire', () => fixture(browser, async f => {
      if (name === 'chromium') await f.context.grantPermissions(['clipboard-read', 'clipboard-write']);
      else await f.context.grantPermissions(['clipboard-read']);
      await f.page.evaluate(value => {
        const button = document.createElement('button'); button.id = 'seed'; button.textContent = 'Copy';
        Object.assign(button.style, { position: 'fixed', top: '10px', zIndex: 5 }); document.body.append(button);
        button.addEventListener('click', () => { window.seeded = navigator.clipboard.writeText(value); });
      }, text);
      await f.page.locator('#seed').tap(); await f.page.evaluate(() => window.seeded);
      await f.tap(); await eventually(() => f.pastes.length === 1);
      await f.page.clock.runFor(1600); await f.state('idle'); assert.deepEqual(f.pastes, [text]);
    }, { native: true }));
    if (name === 'chromium') await t.test('tests that real browser Block permission shows Paste blocked', () => fixture(browser, async f => {
      await f.context.grantPermissions([], { origin: 'https://viewer.test' });
      assert.equal(await f.page.evaluate(async () => (await navigator.permissions.query({ name: 'clipboard-read' })).state), 'denied');
      await f.tap(); await f.state('blocked'); assert.deepEqual(f.pastes, []);
    }, { native: true }));
  });
}
