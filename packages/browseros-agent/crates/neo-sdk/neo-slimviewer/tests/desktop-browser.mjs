// Linux: PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node --test tests/desktop-browser.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const html = readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8');
function record(type, payload = []) {
  const bytes = Buffer.alloc(5 + payload.length);
  bytes.writeUInt32BE(payload.length + 1); bytes[4] = type; bytes.set(payload, 5); return bytes;
}
async function fixture(browser, run, { flags = 14, mac = false, native = false } = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, serviceWorkers: 'block' });
  const page = await context.newPage(), received = [], sockets = [], errors = [], listeners = new Set();
  page.setDefaultTimeout(5000);
  page.on('pageerror', error => errors.push(error.message));
  await context.route('**/*', route => route.fulfill({ contentType: 'text/html', body: html }));
  await context.routeWebSocket('**/slim', socket => {
    sockets.push(socket);
    socket.onMessage(bytes => { received.push(Buffer.from(bytes).subarray(4)); for (const listener of listeners) listener(); });
    socket.send(record(1, [1, flags]));
    const picture = Buffer.alloc(16);
    picture.writeUInt32BE(1); picture[4] = 1; picture.writeUInt16BE(1920, 5); picture.writeUInt16BE(1080, 7);
    picture.set([0, 0, 1, 0x67, 0x42, 0xc0, 0x1f], 9); socket.send(record(2, picture));
  });
  await context.addInitScript(({ mac, native }) => {
    if (mac) {
      Object.defineProperty(navigator, 'platform', { value: 'MacIntel' });
      Object.defineProperty(navigator, 'userAgentData', { value: { platform: 'macOS' } });
    }
    window.VideoDecoder = class {
      static async isConfigSupported() { return { supported: true }; }
      constructor({ output }) { this.output = output; this.state = 'unconfigured'; }
      configure() { this.state = 'configured'; }
      decode(chunk) {
        const frame = document.createElement('canvas'); frame.width = 1920; frame.height = 1080;
        Object.assign(frame, { timestamp: chunk.timestamp, displayWidth: 1920, displayHeight: 1080, close() {} });
        this.output(frame);
      }
      close() { this.state = 'closed'; }
    };
    window.EncodedVideoChunk = class { constructor(value) { Object.assign(this, value); } };
    window.clipboardProbe = { reads: [], reservations: [], writes: [], text: 'local 日本 👋', denied: false };
    if (!native) Object.defineProperty(navigator, 'clipboard', { value: {
      readText() { clipboardProbe.reads.push(navigator.userActivation.isActive); return Promise.resolve(clipboardProbe.text); },
      async write(items) {
        clipboardProbe.reservations.push(navigator.userActivation.isActive);
        if (clipboardProbe.denied) throw new DOMException('Denied', 'NotAllowedError');
        const value = await (await items[0].getType('text/plain')).text();
        clipboardProbe.writes.push(value);
      },
      async writeText(text) {
        if (clipboardProbe.denied) throw new DOMException('Denied', 'NotAllowedError');
        clipboardProbe.writes.push(text);
      },
    } });
  }, { mac, native });
  try {
    await page.goto('https://desktop.test/');
    await page.locator('#cover').waitFor({ state: 'hidden' });
    const messages = type => received.filter(r => r[0] === type);
    const keys = () => messages(0x21).map(r => [r.readUInt32BE(1), r[5]]);
    const waitForWire = predicate => new Promise((resolve, reject) => {
      const timer = setTimeout(() => { listeners.delete(check); reject(new Error('Expected wire records did not arrive')); }, 5000);
      const check = () => { if (predicate()) { clearTimeout(timer); listeners.delete(check); resolve(); } };
      listeners.add(check); check();
    });
    const waitMessages = (type, count) => waitForWire(() => messages(type).length >= count);
    const press = async key => { await page.keyboard.press(key); await page.evaluate(() => new Promise(requestAnimationFrame)); };
    await run({ page, context, received, messages, keys, sockets, press, waitMessages, waitForWire });
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  test(`${name}: desktop input, clipboard and ownership`, async t => {
    const browser = await engine.launch(); t.after(() => browser.close());
    await t.test('fits a 1920x1080 screen and maps every mouse button, drag and double click', () => fixture(browser, async f => {
      const box = await f.page.locator('#screen').boundingBox();
      assert.deepEqual(box, { x: 0, y: 40, width: 1280, height: 720 });
      assert.equal(await f.page.locator('#controls').isVisible(), false);
      await f.page.mouse.click(640, 400);
      await f.waitForWire(() => f.messages(0x20).some(r => r[5] === 1) && f.messages(0x20).at(-1)[5] === 0);
      assert.equal(await f.page.evaluate(() => document.activeElement.id), 'noVNC_keyboardinput');
      let points = f.messages(0x20);
      assert.deepEqual(points.slice(-2).map(r => [r.readUInt16BE(1), r.readUInt16BE(3), r[5]]), [[960, 540, 1], [960, 540, 0]]);
      await f.page.mouse.click(640, 400, { button: 'right' });
      await f.page.mouse.click(640, 400, { button: 'middle' });
      await f.waitForWire(() => f.messages(0x20).some(r => r[5] === 2));
      assert.ok(f.messages(0x20).some(r => r[5] === 4)); assert.ok(f.messages(0x20).some(r => r[5] === 2));
      const before = f.messages(0x20).filter(r => r[5] === 1).length;
      await f.page.mouse.dblclick(640, 400);
      await f.waitForWire(() => f.messages(0x20).filter(r => r[5] === 1).length - before === 2);
      assert.equal(f.messages(0x20).filter(r => r[5] === 1).length - before, 2);
      await f.page.mouse.down(); await f.page.mouse.move(800, 500, { steps: 4 }); await f.page.mouse.up();
      await f.waitForWire(() => { const r = f.messages(0x20).at(-1); return r?.readUInt16BE(1) === 1200 && r?.readUInt16BE(3) === 690 && r?.[5] === 0; });
      points = f.messages(0x20);
      assert.deepEqual(points.slice(-2).map(r => [r.readUInt16BE(1), r.readUInt16BE(3), r[5]]), [[1200, 690, 1], [1200, 690, 0]]);
    }));
    await t.test('forwards physical keys and releases held keys and mouse on blur', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100);
      for (const key of ['a', 'Shift+A', 'F1', 'F12', 'PageUp', 'PageDown', 'Home', 'End', 'Insert', 'Delete', 'ArrowLeft', 'Tab', 'Escape']) await f.press(key);
      const symbols = new Set(f.keys().filter(([, down]) => down).map(([sym]) => sym));
      for (const sym of [97, 65, 0xffbe, 0xffc9, 0xff55, 0xff56, 0xff50, 0xff57, 0xff63, 0xffff, 0xff51, 0xff09, 0xff1b]) assert.ok(symbols.has(sym), sym.toString(16));
      await f.page.keyboard.down('Control'); await f.page.keyboard.down('b'); await f.page.mouse.down();
      await f.page.evaluate(() => dispatchEvent(new Event('blur')));
      await f.waitForWire(() => JSON.stringify(f.keys().slice(-2)) === JSON.stringify([[98, 0], [0xffe3, 0]]));
      assert.deepEqual(f.keys().slice(-2), [[98, 0], [0xffe3, 0]]);
      assert.equal(f.messages(0x20).at(-1)[5], 0);
      await f.page.keyboard.up('b'); await f.page.keyboard.up('Control'); await f.page.mouse.up();
    }));
    await t.test('maps only the listed Command shortcuts, retaining a real Meta for others', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100);
      for (const letter of ['a', 'z', 'y', 'f', 'l', 't', 'w', 'r']) {
        f.received.length = 0;
        await f.press(`Meta+${letter}`);
        assert.deepEqual(f.keys(), [[0xffe3, 1], [letter.charCodeAt(0), 1], [letter.charCodeAt(0), 0], [0xffe3, 0]]);
      }
      f.received.length = 0; await f.press('Meta+ArrowLeft');
      assert.deepEqual(f.keys(), [[0xffeb, 1], [0xff51, 1], [0xff51, 0], [0xffeb, 0]]);
    }, { mac: true }));
    await t.test('releases the previous keysym when Shift changes a repeating physical key', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100);
      await f.page.keyboard.down('a'); await f.page.keyboard.down('Shift'); await f.page.keyboard.down('A');
      await f.page.keyboard.up('A'); await f.page.keyboard.up('Shift');
      await f.waitMessages(0x21, 6);
      assert.deepEqual(f.keys(), [[97, 1], [0xffe1, 1], [97, 0], [65, 1], [65, 0], [0xffe1, 0]]);
    }));
    await t.test('starts paste inside Ctrl/Cmd+V and sends PASTE exactly once', async () => {
      for (const mac of [false, true]) await fixture(browser, async f => {
        await f.page.mouse.click(100, 100);
        await f.press(`${mac ? 'Meta' : 'Control'}+v`);
        await f.page.waitForFunction(() => clipboardProbe.reads.length === 1);
        assert.deepEqual(await f.page.evaluate(() => clipboardProbe.reads), [true]);
        assert.deepEqual(f.messages(0x22).map(r => r.subarray(1).toString()), ['local 日本 👋']);
        assert.ok(!f.keys().some(([sym]) => sym === 118));
      }, { mac });
    });
    await t.test('reserves copy during key activation and accepts Unicode from the server', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100);
      assert.equal(f.messages(0x24).length, 1);
      for (const key of ['c', 'x']) {
        await f.press(`Meta+${key}`);
        await f.page.waitForFunction(() => clipboardProbe.reservations.length > clipboardProbe.writes.length);
        assert.ok((await f.page.evaluate(() => clipboardProbe.reservations)).every(Boolean));
        const text = `Neo ${key}: café 日本 👩🏽‍💻`;
        f.sockets[0].send(record(3, Buffer.from(text)));
        await f.page.waitForFunction(text => clipboardProbe.writes.at(-1) === text, text);
      }
      f.sockets[0].send(record(3, Buffer.from('context-menu copy')));
      await f.page.waitForFunction(() => clipboardProbe.writes.at(-1) === 'context-menu copy');
    }, { mac: true }));
    await t.test('a real reserved clipboard write can paste into a local textarea', () => fixture(browser, async f => {
      await f.context.grantPermissions(name === 'chromium' ? ['clipboard-read', 'clipboard-write'] : ['clipboard-read']);
      await f.page.mouse.click(100, 100); await f.press('Control+c');
      const text = 'Copied from Neo: café 日本 👩🏽‍💻';
      f.sockets[0].send(record(3, Buffer.from(text)));
      await f.page.waitForFunction(async text => await navigator.clipboard.readText() === text, text);
      await f.page.evaluate(() => {
        const input = document.createElement('textarea'); input.id = 'local';
        Object.assign(input.style, { position: 'fixed', inset: '10px', width: '500px', height: '200px', zIndex: 10 });
        document.body.append(input);
      });
      await f.page.locator('#local').click(); await f.press(process.platform === 'darwin' ? 'Meta+v' : 'Control+v');
      assert.equal(await f.page.locator('#local').inputValue(), text);
      assert.equal(f.messages(0x22).length, 0, 'local paste must not go back to Neo');
    }, { native: true }));
    await t.test('cancels a pending copy on handoff and never writes a late clipboard', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100); await f.press('Control+c');
      f.sockets[0].send(record(1, [1, 1]));
      await f.page.waitForFunction(() => !slimviewer.connected);
      assert.deepEqual(await f.page.evaluate(() => clipboardProbe.writes), []);
      assert.equal(await f.page.locator('#clipboard-status').textContent(), '');
    }));
    await t.test('reports clipboard refusal without a dialog', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100);
      await f.page.evaluate(() => { clipboardProbe.denied = true; });
      await f.press('Control+c');
      await f.page.waitForFunction(() => document.getElementById('clipboard-status').textContent.includes('blocked'));
      assert.deepEqual(await f.page.evaluate(() => clipboardProbe.writes), []);
    }));
    await t.test('sends smooth fractional horizontal and vertical pixel deltas at fitted scale', () => fixture(browser, async f => {
      await f.page.mouse.move(640, 400);
      await f.page.locator('#screen').dispatchEvent('wheel', { clientX: 640, clientY: 400, deltaX: 1.25, deltaY: 2.5, deltaMode: 0 });
      await f.page.evaluate(() => new Promise(requestAnimationFrame));
      await f.waitMessages(0x25, 1);
      const wheel = f.messages(0x25).at(-1);
      assert.ok(wheel); assert.equal(wheel.length, 13);
      assert.deepEqual([wheel.readUInt16BE(1), wheel.readUInt16BE(3), wheel.readFloatBE(5), wheel.readFloatBE(9)], [960, 540, 1.875, 3.75]);
      await f.page.locator('#screen').dispatchEvent('wheel', { clientX: 640, clientY: 400, deltaX: 0, deltaY: 2, deltaMode: 1, shiftKey: true });
      await f.page.evaluate(() => new Promise(requestAnimationFrame));
      await f.waitMessages(0x25, 2);
      assert.equal(f.messages(0x25).at(-1).readFloatBE(5), 120);
      assert.equal(f.messages(0x25).at(-1).readFloatBE(9), 0);
    }));
    await t.test('falls back to both wheel axes on older servers', () => fixture(browser, async f => {
      await f.page.locator('#screen').dispatchEvent('wheel', { clientX: 640, clientY: 400, deltaX: -40, deltaY: 40, deltaMode: 0 });
      await f.page.evaluate(() => new Promise(requestAnimationFrame));
      await f.waitMessages(0x20, 4);
      assert.equal(f.messages(0x25).length, 0);
      assert.deepEqual(f.messages(0x20).map(r => r[5]), [32, 0, 16, 0]);
    }, { flags: 0 }));
    await t.test('commits IME and emoji as TYPE without leaking preedit', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100); f.received.length = 0;
      await f.page.locator('#noVNC_keyboardinput').dispatchEvent('compositionstart');
      await f.page.evaluate(() => {
        const input = document.getElementById('noVNC_keyboardinput');
        input.value += '日本'; input.dispatchEvent(new InputEvent('input', { isComposing: true, bubbles: true }));
      });
      assert.equal(f.messages(0x23).length, 0);
      await f.page.locator('#noVNC_keyboardinput').dispatchEvent('compositionend');
      await f.page.keyboard.insertText(' 👩🏽‍💻');
      await f.waitMessages(0x23, 2);
      assert.deepEqual(f.messages(0x23).map(r => r.subarray(1).toString()), ['日本', ' 👩🏽‍💻']);
      assert.equal(f.keys().length, 0);
    }));
    await t.test('spectators never request clipboard, write clipboard or send input', () => fixture(browser, async f => {
      await f.page.mouse.click(100, 100); await f.press('Control+c'); await f.press('Control+v');
      f.sockets[0].send(record(3, Buffer.from('private')));
      await f.page.evaluate(() => new Promise(requestAnimationFrame));
      assert.equal(f.messages(0x24).length, 0); assert.equal(f.keys().length, 0); assert.equal(f.messages(0x22).length, 0);
      assert.deepEqual(await f.page.evaluate(() => [clipboardProbe.reads, clipboardProbe.writes]), [[], []]);
    }, { flags: 1 }));
    await t.test('rejects oversized and malformed remote clipboard records', async () => {
      for (const data of [Buffer.alloc(65537, 65), Buffer.from([0xff]), Buffer.from('a\0b')]) await fixture(browser, async f => {
        f.sockets[0].send(record(3, data));
        await f.page.waitForFunction(() => !slimviewer.connected);
        assert.deepEqual(await f.page.evaluate(() => clipboardProbe.writes), []);
      });
    });
  });
}
