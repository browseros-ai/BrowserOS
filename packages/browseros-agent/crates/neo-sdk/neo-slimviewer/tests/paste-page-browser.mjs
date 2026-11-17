// PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node --test tests/paste-page-browser.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

const { chromium, webkit, devices } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const html = readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8');
const adapter = readFileSync(new URL('../web/ui.js', import.meta.url), 'utf8');
const width = 390, height = 844;
function record(type, payload) {
  const bytes = Buffer.alloc(5 + payload.length);
  bytes.writeUInt32BE(1 + payload.length); bytes[4] = type; bytes.set(payload, 5);
  return bytes;
}
function fields(boxes, focused = 255, w = width, h = height) {
  const bytes = Buffer.alloc(6 + boxes.length * 8);
  bytes.writeUInt16BE(w); bytes.writeUInt16BE(h, 2); bytes[4] = focused; bytes[5] = boxes.length;
  boxes.flat().forEach((n, i) => bytes.writeUInt16BE(n, 6 + i * 2));
  return bytes;
}
async function eventually(check) {
  const deadline = Date.now() + 2500;
  while (!await check() && Date.now() < deadline) await delay(10);
  assert.ok(await check(), 'expected viewer behavior');
}

async function fixture(browser, run, { flags = 62, framed = false } = {}) {
  const context = await browser.newContext({ ...devices['iPhone 13'], viewport: { width, height }, serviceWorkers: 'block' });
  const page = await context.newPage(), sockets = [], messages = [], errors = [];
  page.setDefaultTimeout(3000);
  page.on('pageerror', error => errors.push(error.message));
  await page.clock.install();
  await context.route('**/*', route => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({ contentType: path === '/ui.js' ? 'text/javascript' : 'text/html', body:
      path === '/ui.js' ? adapter : path === '/host' ? '<meta name="viewport" content="width=device-width,initial-scale=1"><style>body{margin:0}iframe{width:390px;height:844px;border:0}</style><iframe src="/?parents=https%3A%2F%2Fviewer.test"></iframe>' : html });
  });
  await context.routeWebSocket('**/slim', socket => {
    sockets.push(socket);
    socket.onMessage(message => messages.push(Buffer.from(message)));
    socket.send(record(1, [1, flags]));
    const picture = Buffer.alloc(16);
    picture.writeUInt32BE(1); picture[4] = 1;
    picture.writeUInt16BE(width, 5); picture.writeUInt16BE(height, 7);
    picture.set([0, 0, 1, 0x67, 0x42, 0xc0, 0x1f], 9);
    socket.send(record(2, picture));
  });
  await context.addInitScript(() => {
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
    window.probe = { pending: [], outcomes: [], reads: [], inTap: false, permission: 'prompt' };
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { readText() {
      probe.reads.push({ inTap: probe.inTap, active: navigator.userActivation.isActive });
      return new Promise((resolve, reject) => probe.pending.push({ resolve, reject }));
    } } });
    Object.defineProperty(navigator, 'permissions', { configurable: true, value: {
      async query() { return { state: probe.permission }; },
    } });
  });
  try {
    await page.goto('https://viewer.test/' + (framed ? 'host' : ''));
    const viewer = framed ? page.frames().find(frame => frame.parentFrame()) : page;
    await viewer.waitForFunction(() => window.__stats?.frames === 1);
    await page.clock.runFor(2100);
    await viewer.locator('#cover').waitFor({ state: 'hidden' });
    await viewer.evaluate(async () => {
      const { pasteFromPill } = await import('/ui.js');
      const button = document.createElement('button'); button.id = 'call-paste'; button.textContent = 'Paste API';
      Object.assign(button.style, { position: 'fixed', top: '4px', left: '4px', zIndex: 5 }); document.body.append(button);
      button.onclick = event => {
        probe.inTap = event.isTrusted;
        const index = probe.outcomes.push(null) - 1;
        (probe.pill ? pasteFromPill() : slimviewer.paste()).then(outcome => { probe.outcomes[index] = outcome; });
        probe.inTap = false;
      };
    });
    const send = (type, payload) => sockets.at(-1).send(record(type, payload));
    const result = (id, outcome) => {
      const bytes = Buffer.alloc(5); bytes.writeUInt32BE(id); bytes[4] = outcome; send(4, bytes);
    };
    const tap = () => viewer.locator('#call-paste').tap();
    const settle = (index, value, reject = false) => viewer.evaluate(({ index, value, reject }) => {
      const read = probe.pending[index];
      reject ? read.reject(new DOMException('Denied', 'NotAllowedError')) : read.resolve(value);
    }, { index, value, reject });
    const pastes = () => messages.filter(m => [0x22, 0x26].includes(m[4])).map(m => ({
      type: m[4], id: m[4] === 0x26 ? m.readUInt32BE(5) : null,
      text: new TextDecoder('utf-8', { fatal: true }).decode(m.subarray(m[4] === 0x26 ? 9 : 5)),
    }));
    const outcomes = () => viewer.evaluate(() => probe.outcomes);
    const label = () => viewer.locator('#paste-btn').textContent();
    const visible = area => page.evaluate(area => {
      document.querySelector('iframe').contentWindow.postMessage({ type: 'host-pill', v: 1, kind: 'bounds', visible: area }, location.origin);
    }, { left: 0, top: 0, right: 390, bottom: area });
    const hit = (x, y) => viewer.locator('#screen').tap({ position: { x, y } });
    const focused = () => viewer.evaluate(() => document.activeElement.id === 'noVNC_keyboardinput');
    await run({ page, viewer, sockets, messages, send, result, tap, settle, pastes, outcomes, label, visible, hit, focused });
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  test(`tests that ${name} follows the checked paste and field contract`, async t => {
    const browser = await engine.launch();
    t.after(() => browser.close());
    await t.test('tests that fast pastes send in tap order and resolve their own results', () => fixture(browser, async f => {
      await f.tap(); await f.tap();
      assert.deepEqual(await f.viewer.evaluate(() => probe.reads), [{ inTap: true, active: true }, { inTap: true, active: true }]);
      await f.settle(1, 'second'); assert.deepEqual(f.pastes(), []);
      await f.settle(0, 'first');
      await eventually(() => f.pastes().length === 2);
      assert.deepEqual(f.pastes(), [{ type: 0x26, id: 1, text: 'first' }, { type: 0x26, id: 2, text: 'second' }]);
      f.result(2, 0); await eventually(async () => (await f.outcomes())[1] === 'pasted');
      assert.equal(await f.label(), 'Pasted');
      f.result(1, 1); await eventually(async () => (await f.outcomes())[0] === 'try-again');
      assert.equal(await f.label(), 'Pasted');
      await f.page.clock.runFor(1501); assert.equal(await f.label(), 'Paste');
    }));
    await t.test('tests that every result and pill outcome uses the public names', () => fixture(browser, async f => {
      await f.viewer.evaluate(() => { probe.pill = true; });
      for (const [i, code] of [0, 1, 2, 3, 4, 255].entries()) {
        await f.tap(); await f.settle(i, 'value'); await eventually(() => f.pastes().length === i + 1);
        f.result(i + 1, code);
        const outcome = ['pasted', 'try-again', 'try-again', 'blocked', 'sent', 'sent'][i];
        await eventually(async () => (await f.outcomes())[i] === outcome);
        assert.equal(await f.label(), ['Pasted', 'Try again', 'Try again', 'Paste blocked', 'Paste', 'Paste'][i]);
        if (outcome === 'try-again') {
          await f.page.clock.runFor(2900); assert.equal(await f.label(), 'Try again');
          await f.page.clock.runFor(101); assert.equal(await f.label(), 'Paste');
        }
      }
      await f.tap(); await f.settle(6, '', true);
      await eventually(async () => (await f.outcomes())[6] === 'cancelled');
      assert.equal(await f.label(), 'Tap again');
    }));
    await t.test('tests that native paste waits for the earlier clipboard read', () => fixture(browser, async f => {
      await f.tap();
      await f.viewer.evaluate(() => {
        const data = new DataTransfer(); data.setData('text/plain', 'native');
        document.getElementById('noVNC_keyboardinput').dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
      });
      assert.deepEqual(f.pastes(), []);
      await f.settle(0, 'read'); await eventually(() => f.pastes().length === 2);
      assert.deepEqual(f.pastes().map(p => p.text), ['read', 'native']);
      f.result(2, 0); await eventually(async () => await f.label() === 'Pasted');
      f.result(1, 3); await eventually(async () => (await f.outcomes())[0] === 'blocked');
      assert.equal(await f.label(), 'Pasted');
    }));
    await t.test('tests that a rejected or empty read releases its slot without suppressing another result', () => fixture(browser, async f => {
      await f.tap(); await f.tap(); await f.settle(1, 'kept'); await f.settle(0, '', true);
      await eventually(() => f.pastes().length === 1); f.result(1, 0);
      await eventually(async () => (await f.outcomes())[1] === 'pasted');
      assert.deepEqual(await f.outcomes(), ['tap-again', 'pasted']);
      await f.tap(); await f.tap(); await f.settle(3, 'also kept'); await f.settle(2, '');
      await eventually(() => f.pastes().length === 2); f.result(2, 0);
      await eventually(async () => (await f.outcomes())[3] === 'pasted');
      assert.deepEqual(await f.outcomes(), ['tap-again', 'pasted', 'empty', 'pasted']);
    }));
    await t.test('tests that timeout and disconnect resolve sent without sending an old read after reconnect', () => fixture(browser, async f => {
      await f.tap(); await f.settle(0, 'pending'); await eventually(() => f.pastes().length === 1);
      f.result(999, 0); assert.deepEqual(await f.outcomes(), [null]);
      await f.page.clock.runFor(5001); assert.deepEqual(await f.outcomes(), ['sent']);
      f.result(1, 1); assert.equal(await f.label(), 'Paste');
      await f.tap(); await f.settle(1, 'pending result'); await eventually(() => f.pastes().length === 2);
      await f.tap(); await f.sockets[0].close();
      await eventually(async () => (await f.outcomes()).every(Boolean));
      assert.deepEqual(await f.outcomes(), ['sent', 'sent', 'sent']);
      await f.page.clock.runFor(2400); await f.viewer.waitForFunction(() => slimviewer.connected);
      await f.settle(2, 'old connection'); assert.equal(f.pastes().length, 2);
      await f.tap(); await f.settle(3, 'new connection'); await eventually(() => f.pastes().length === 3);
      assert.equal(f.pastes()[2].id, 1);
    }));
    await t.test('tests that a server without bit 16 uses legacy PASTE and resolves sent', () => fixture(browser, async f => {
      await f.tap(); await f.tap(); await f.settle(1, 'second'); await f.settle(0, 'first');
      await eventually(async () => (await f.outcomes()).every(Boolean));
      assert.deepEqual(await f.outcomes(), ['sent', 'sent']);
      assert.deepEqual(f.pastes(), [{ type: 0x22, id: null, text: 'first' }, { type: 0x22, id: null, text: 'second' }]);
      assert.equal(f.messages.some(m => m[4] === 0x27), false);
    }, { flags: 14 }));
    await t.test('tests that NUL is blocked and the byte cap preserves Unicode', () => fixture(browser, async f => {
      await f.tap(); await f.settle(0, 'bad\0text');
      await eventually(async () => (await f.outcomes())[0] === 'blocked'); assert.deepEqual(f.pastes(), []);
      await f.tap(); await f.settle(1, 'a'.repeat(65535) + '👍');
      await eventually(() => f.pastes().length === 1);
      assert.equal(f.pastes()[0].text.length, 65535);
    }));
    await t.test('tests that FIELDS subscribes and opens the keyboard on a field tap', () => fixture(browser, async f => {
      assert.ok(f.messages.some(m => m[4] === 0x27 && m.length === 5));
      f.send(5, fields([[20, 650, 300, 40]], 0)); await f.hit(40, 670);
      assert.equal(await f.focused(), true);
    }));
    await t.test('tests that wrong-sized fields are ignored and recent previous fields can be hit', () => fixture(browser, async f => {
      f.send(5, fields([[20, 650, 300, 40]], 0, 400, 844)); await f.hit(40, 670);
      assert.equal(await f.focused(), false);
      f.send(5, fields([[20, 650, 300, 40]], 0));
      await f.page.clock.runFor(20);
      f.send(5, fields([[20, 500, 300, 40]], 0)); await f.hit(40, 670);
      assert.equal(await f.focused(), true);
      await f.viewer.locator('#kbd-btn').tap(); await f.page.clock.runFor(601); await f.hit(40, 670);
      assert.equal(await f.focused(), false);
    }));
    await t.test('tests that fresh FIELDS win and stale FIELDS fall back to parent geometry', () => fixture(browser, async f => {
      await f.page.evaluate(() => document.querySelector('iframe').contentWindow.postMessage({ type: 'host-keyboard', geometry: { width: 390, height: 844, boxes: [[20, 650, 300, 40]] } }, location.origin));
      f.send(5, fields([])); await f.hit(40, 670); assert.equal(await f.focused(), false);
      await f.page.clock.runFor(3001); await f.hit(40, 670); assert.equal(await f.focused(), true);
    }, { framed: true }));
    await t.test('tests that focused fields pan above parent bounds and taps still map to screen pixels', () => fixture(browser, async f => {
      f.send(5, fields([[20, 650, 300, 40]], 0)); await f.hit(40, 670); await f.visible(500);
      const geometry = () => f.viewer.evaluate(() => {
        const canvas = document.getElementById('screen'), rect = canvas.getBoundingClientRect();
        return { top: rect.top, bottom: rect.top + 690 * rect.height / canvas.height, translate: canvas.style.translate };
      });
      await eventually(async () => Math.abs((await geometry()).bottom - 488) < 1);
      const before = await geometry(); await f.visible(500); assert.deepEqual(await geometry(), before);
      const count = f.messages.filter(m => m[4] === 0x20).length;
      await f.hit(40, 670);
      await eventually(() => f.messages.filter(m => m[4] === 0x20).length > count);
      const pointers = f.messages.filter(m => m[4] === 0x20);
      assert.equal(pointers.at(-1).readUInt16BE(7), 670);
      await f.visible(844); await eventually(async () => (await geometry()).top === 0);
      await f.visible(500); await eventually(async () => (await geometry()).top < 0);
      await f.viewer.locator('#kbd-btn').tap(); await eventually(async () => (await geometry()).top === 0);
      await f.hit(40, 670); await f.visible(500); await eventually(async () => (await geometry()).top < 0);
      await f.sockets[0].close(); await eventually(async () => (await geometry()).top === 0);
    }, { framed: true }));
    await t.test('tests that only valid bounds from the allowed parent can pan the canvas', () => fixture(browser, async f => {
      f.send(5, fields([[20, 650, 300, 40]], 0)); await f.hit(40, 670);
      await f.viewer.evaluate(() => {
        const data = { type: 'host-pill', v: 1, kind: 'bounds', visible: { left: 0, top: 0, right: 390, bottom: 500 } };
        for (const event of [
          { source: parent, origin: 'https://untrusted.test', data },
          { source: window, origin: 'https://viewer.test', data },
          { source: parent, origin: 'https://viewer.test', data: { ...data, v: 2 } },
          { source: parent, origin: 'https://viewer.test', data: { ...data, visible: { ...data.visible, bottom: NaN } } },
        ]) dispatchEvent(new MessageEvent('message', event));
      });
      assert.equal(await f.viewer.locator('#screen').evaluate(canvas => canvas.getBoundingClientRect().top), 0);
      await f.visible(500);
      await eventually(async () => await f.viewer.locator('#screen').evaluate(canvas => canvas.getBoundingClientRect().top) === -202);
    }, { framed: true }));
    await t.test('tests that legacy parent boxes pan and tall fields keep their top visible', () => fixture(browser, async f => {
      await f.page.evaluate(() => document.querySelector('iframe').contentWindow.postMessage({ type: 'host-keyboard', geometry: { width: 390, height: 844, boxes: [[20, 300, 300, 500]] } }, location.origin));
      await f.hit(40, 450); await f.visible(500);
      await eventually(async () => await f.viewer.locator('#screen').evaluate(canvas => canvas.getBoundingClientRect().top + 300) === 12);
    }, { framed: true, flags: 14 }));
    await t.test('tests that standalone visual viewport changes pan the last tapped field', () => fixture(browser, async f => {
      f.send(5, fields([[20, 650, 300, 40]])); await f.hit(40, 670);
      await f.viewer.evaluate(() => {
        Object.defineProperty(visualViewport, 'height', { configurable: true, value: 500 });
        visualViewport.dispatchEvent(new Event('resize'));
      });
      await eventually(async () => await f.viewer.evaluate(() => document.getElementById('screen').getBoundingClientRect().top) === -202);
      await f.viewer.evaluate(() => {
        Object.defineProperty(visualViewport, 'height', { configurable: true, value: 844 });
        visualViewport.dispatchEvent(new Event('resize'));
      });
      assert.equal(await f.viewer.locator('#screen').evaluate(canvas => canvas.getBoundingClientRect().top), 0);
    }));
    for (const [kind, payload] of [
      [4, Buffer.alloc(4)], [4, Buffer.alloc(6)],
      [5, Buffer.alloc(5)], [5, Buffer.from([1, 134, 3, 76, 255, 129])],
      [5, fields([[20, 650, 300, 40]], 1)], [5, fields([[380, 650, 20, 40]])],
    ]) {
      await t.test('tests that malformed result and field records close the connection', () => fixture(browser, async f => {
        f.send(kind, payload); await f.viewer.waitForFunction(() => !slimviewer.connected);
      }));
    }
  });
}
