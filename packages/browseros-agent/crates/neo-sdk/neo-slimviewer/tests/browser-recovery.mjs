import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const page = [...readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8').matchAll(/<script>([\s\S]*?)<\/script>/g)].map(match => match[1]).join('\n');
const adapter = readFileSync(new URL('../web/ui.js', import.meta.url), 'utf8').replace('export default', 'globalThis.UI =').replace('export function pasteFromPill', 'function pasteFromPill');
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

function record(type, payload = []) {
  const out = new Uint8Array(5 + payload.length);
  new DataView(out.buffer).setUint32(0, payload.length + 1);
  out[4] = type; out.set(payload, 5);
  return out;
}

function frame(seq, key = true, width = 640, height = 480) {
  const payload = new Uint8Array(16);
  const view = new DataView(payload.buffer);
  view.setUint32(0, seq); payload[4] = Number(key);
  view.setUint16(5, width); view.setUint16(7, height);
  payload.set([0, 0, 1, 0x67, 0x42, 0xc0, 0x1f], 9);
  return record(2, payload);
}

function browser({ support = async () => ({ supported: true }), noDecoder = false, configureError = null, output = true,
  clipboard = async () => 'hello', failDecodeSeq = null, viewOnlyQuery = false, noSegmenter = false,
  width = 640, height = 480, scale = 1, focused = true } = {}) {
  const messages = [], sockets = [], decoders = [], timers = new Map(), elements = new Map(), listeners = new Map();
  const focus = [];
  let nextTimer = 0;
  const classes = new Set();
  const classList = { add: c => classes.add(c), remove: c => classes.delete(c), toggle: (c, enabled) => enabled ? classes.add(c) : classes.delete(c) };
  function listen(handlers, type, fn) {
    const previous = handlers.get(type);
    handlers.set(type, previous ? event => { previous(event); fn(event); } : fn);
  }
  function element() {
    return { style: {}, dataset: {}, classList, hidden: false, value: '', width: 640, height: 480, textContent: '', listeners: new Map(),
      addEventListener(type, fn) { listen(this.listeners, type, fn); }, setAttribute() {}, setSelectionRange() {}, focus() {}, blur() {},
      getContext: () => ({ drawImage() {} }), getBoundingClientRect: () => ({ left: 0, top: 0, width: 640, height: 480 }) };
  }
  for (const id of ['screen', 'noVNC_keyboardinput', 'cover', 'controls', 'kbd-btn', 'paste-btn']) elements.set(id, element());
  class Socket {
    static OPEN = 1;
    constructor() { this.readyState = 1; this.sent = []; sockets.push(this); }
    send(bytes) { this.sent.push(new Uint8Array(bytes)); }
    close(code) {
      if (code !== undefined && code !== 1000 && !(code >= 3000 && code <= 4999)) throw new Error('Invalid WebSocket close code');
      if (this.readyState !== 3) { this.readyState = 3; this.onclose?.(); }
    }
    receive(bytes) { this.onmessage?.({ data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }); }
  }
  class Decoder {
    static isConfigSupported = support;
    constructor(callbacks) { this.callbacks = callbacks; this.state = 'unconfigured'; this.decoded = []; decoders.push(this); }
    configure(config) { if (configureError) throw configureError; this.config = config; this.state = 'configured'; }
    decode(chunk) {
      if (chunk.timestamp === failDecodeSeq) throw new Error('Corrupt frame');
      this.decoded.push(chunk);
      if (output) this.callbacks.output({ timestamp: chunk.timestamp, displayWidth: this.config.codedWidth, displayHeight: this.config.codedHeight, close() {} });
    }
    close() { this.state = 'closed'; }
  }
  const context = vm.createContext({ URLSearchParams, Uint8Array, DataView, TextEncoder, Map, Set, console, EventTarget, Event, CustomEvent,
    WebSocket: Socket, EncodedVideoChunk: class { constructor(fields) { Object.assign(this, fields); } },
    location: { search: '?parents=https%3A%2F%2Fparent.test&path=stream' + (viewOnlyQuery ? '&view_only=1' : ''), protocol: 'http:', host: 'viewer.test' },
    parent: { postMessage(message, origin) { messages.push({ ...message, origin }); } },
    document: { documentElement: { classList }, getElementById: id => elements.get(id) || null, addEventListener() {}, hasFocus: () => focused },
    isSecureContext: true, navigator: { clipboard: { readText: clipboard } },
    matchMedia: () => ({ matches: false }), addEventListener(type, listener) { listen(listeners, type, listener); }, performance: { now: () => 0 },
    innerWidth: width, innerHeight: height, devicePixelRatio: scale,
    focus() { focus.push('focus'); focused = true; },
    setTimeout(fn, ms) { const id = ++nextTimer; timers.set(id, { fn, ms }); return id; },
    clearTimeout: id => timers.delete(id), requestAnimationFrame: fn => fn(), cancelAnimationFrame() {}, prompt: () => '' });
  context.window = context;
  if (noSegmenter) context.Intl = {};
  if (!noDecoder) context.VideoDecoder = Decoder;
  vm.runInContext(page, context);
  vm.runInContext(adapter, context);
  function fireTimer() {
    const [id, timer] = [...timers].sort((a, b) => a[1].ms - b[1].ms)[0] || [];
    assert.ok(timer, 'expected a scheduled timer'); timers.delete(id); timer.fn(); return timer.ms;
  }
  return { context, sockets, messages, decoders, timers, elements, classes, fireTimer, focus,
    fire(type, event = {}) { listeners.get(type)(event); },
    message(data) { listeners.get('message')({ source: context.parent, origin: 'https://parent.test', data }); },
    hello(socket = sockets.at(-1), viewOnly = false, typeSupport = true) { socket.receive(record(1, [1, Number(viewOnly) | (typeSupport ? 2 : 0)])); } };
}

test('tests that odd screen sizes settle at the encoder even dimensions', async () => {
  for (const [width, height, scale, encodedWidth, encodedHeight] of [[641, 481, 1, 640, 480], [393, 615, 1.5, 786, 1230]]) {
    const b = browser({ width, height, scale }); b.hello();
    b.message({ type: 'host-sizing' });
    b.sockets[0].receive(frame(1, true, encodedWidth, encodedHeight)); await flush();
    b.fireTimer();
    assert.equal(b.context.slimviewer.state, 'ready');
    assert.equal(b.elements.get('cover').hidden, true);
  }
});

test('tests that a phone between 1x and 2x asks for 2x, the only density above 1 Neo draws', async () => {
  for (const [scale, wanted] of [[1, 1], [1.5, 2], [1.75, 2], [2, 2], [3, 2]]) {
    const b = browser({ width: 390, height: 844, scale }); b.hello();
    b.message({ type: 'host-sizing' });
    const layout = b.messages.filter(m => m.type === 'host-layout').at(-1);
    assert.equal(layout.scale, wanted, `DPR ${scale}`);
    assert.deepEqual([...layout.css], [390, 844]);
  }
  // A 1.5x phone's 1.5x picture is not what it asked for; its 2x picture is.
  const b = browser({ width: 390, height: 844, scale: 1.5 }); b.hello();
  b.message({ type: 'host-sizing' });
  b.sockets[0].receive(frame(1, true, 584, 1266)); await flush();
  assert.equal(b.timers.size, 0);
  b.sockets[0].receive(frame(2, true, 780, 1688)); await flush();
  b.fireTimer();
  assert.equal(b.context.slimviewer.state, 'ready');
});

test('tests that a layout Neo confirmed uncovers after 250 ms, an unconfirmed one after 2 s', async () => {
  for (const confirmed of [false, true]) {
    const b = browser({ width: 390, height: 844, scale: 3 }); b.hello();
    b.message({ type: 'host-sizing' });
    b.sockets[0].receive(frame(1, true, 780, 1688)); await flush();
    if (confirmed) b.message({ type: 'host-keyboard', geometry: { width: 780, height: 1688, boxes: [] } });
    assert.equal(b.fireTimer(), confirmed ? 250 : 2000);
    assert.equal(b.context.slimviewer.state, 'ready');
    assert.equal(b.elements.get('cover').hidden, true);
  }
  // Geometry for another size is not a confirmation.
  const b = browser({ width: 390, height: 844, scale: 3 }); b.hello();
  b.message({ type: 'host-sizing' });
  b.sockets[0].receive(frame(1, true, 780, 1688)); await flush();
  b.message({ type: 'host-keyboard', geometry: { width: 1600, height: 1000, boxes: [] } });
  assert.equal(b.fireTimer(), 2000);
});

test('tests that a mismatched framebuffer cannot become ready after rounding', async () => {
  const b = browser({ width: 643, height: 481 }); b.hello();
  b.message({ type: 'host-sizing' });
  b.sockets[0].receive(frame(1)); await flush();
  assert.equal(b.context.slimviewer.state, 'connected');
  assert.equal(b.timers.size, 0);
});

test('tests that missing grapheme support reports unsupported instead of crashing startup', () => {
  const b = browser({ noSegmenter: true });
  if (b.sockets.length) b.hello();
  assert.equal(b.context.slimviewer.state, 'unsupported');
  assert.ok(b.messages.some(m => m.state === 'unsupported'));
  assert.equal(b.context.slimviewer.connected, false);
  assert.equal(b.context.slimviewer.type('secret'), false);
  assert.equal(b.timers.size, 0);
});

test('tests that paste focuses an unfocused frame before reading synchronously', async () => {
  let b;
  b = browser({ focused: false, clipboard: () => {
    assert.deepEqual(b.focus, ['focus']);
    assert.equal(b.context.document.hasFocus(), true);
    return Promise.resolve('framed text');
  } });
  b.hello();
  const pending = b.context.slimviewer.paste();
  assert.deepEqual(b.focus, ['focus']);
  assert.equal(await pending, 'sent');
  assert.equal(new TextDecoder().decode(b.sockets[0].sent[0].subarray(5)), 'framed text');
});

test('tests that paste opens the typing field inside the tap, before the clipboard read', async () => {
  const order = [];
  const b = browser({ clipboard: () => { order.push('read'); return Promise.resolve('pasted'); } });
  const field = b.elements.get('noVNC_keyboardinput');
  field.focus = () => { order.push('keyboard'); b.context.document.activeElement = field; };
  b.hello();
  assert.equal(await b.context.slimviewer.paste(), 'sent');
  assert.deepEqual(order, ['keyboard', 'read']);
  // An open keyboard is left as it is.
  assert.equal(await b.context.slimviewer.paste(), 'sent');
  assert.deepEqual(order, ['keyboard', 'read', 'read']);
});

test('tests that a field tap uses the last field geometry until a newer one arrives', () => {
  const b = browser(); b.hello();
  const canvas = b.elements.get('screen'), opened = [];
  b.elements.get('noVNC_keyboardinput').focus = () => opened.push('keyboard');
  const tap = (x, y) => {
    canvas.listeners.get('pointerdown')({ pointerType: 'touch', pointerId: 1, clientX: x, clientY: y });
    canvas.listeners.get('pointerup')({ pointerType: 'touch', pointerId: 1, type: 'pointerup', clientX: x, clientY: y });
  };
  b.message({ type: 'host-keyboard', geometry: { width: 640, height: 480, boxes: [[100, 100, 200, 40]] } });
  // A resize and a snapshot without geometry clear it, while the canvas still shows that picture.
  b.fire('resize');
  b.message({ type: 'host-keyboard', geometry: null });
  tap(150, 120);
  assert.deepEqual(opened, ['keyboard']);
  tap(10, 10);
  assert.deepEqual(opened, ['keyboard'], 'a tap outside every field opens nothing');
  b.message({ type: 'host-keyboard', geometry: { width: 640, height: 480, boxes: [] } });
  tap(150, 120);
  assert.deepEqual(opened, ['keyboard'], 'a newer geometry replaces it, even one without fields');
  b.message({ type: 'host-keyboard', geometry: { width: 780, height: 1592, boxes: [[100, 100, 200, 40]] } });
  tap(150, 120);
  assert.deepEqual(opened, ['keyboard'], 'boxes for another framebuffer size never apply');
});

test('tests that older binaries receive key pairs for committed text', () => {
  const b = browser(); b.hello(undefined, false, false);
  let inputs = 0; b.context.slimviewer.addEventListener('input', () => inputs++);
  assert.equal(b.context.slimviewer.type('Aé日👍\n\t'), true);
  assert.ok(b.sockets[0].sent.every(m => m[4] === 0x21));
  const keys = b.sockets[0].sent.map(m => [new DataView(m.buffer).getUint32(5), m[9]]);
  assert.deepEqual(keys, [65, 233, 0x10065e5, 0x101f44d, 0xff0d, 0xff09].flatMap(sym => [[sym, 1], [sym, 0]]));
  assert.equal(inputs, 1);
});

test('tests that reconnect negotiates text support again before sending any input', () => {
  const b = browser(); b.hello();
  b.sockets[0].close(); b.fireTimer();
  assert.equal(b.context.slimviewer.type('secret'), false);
  b.hello(undefined, false, false);
  assert.equal(b.context.slimviewer.type('a'), true);
  assert.deepEqual(b.sockets.at(-1).sent.map(m => m[4]), [0x21, 0x21]);
});

test('tests that a supported H.264 frame is drawn and acked through the unchanged pill adapter', async () => {
  const b = browser(); b.hello(undefined, true); b.sockets[0].receive(frame(7)); await flush();
  assert.equal(b.context.__stats.frames, 1);
  assert.equal(b.sockets[0].sent.at(-1)[4], 0x10);
  assert.equal(new DataView(b.sockets[0].sent.at(-1).buffer).getUint32(5), 7);
  assert.equal(b.context.UI.connected, true); assert.equal(b.context.UI.rfb.viewOnly, true);
});

test('tests that unavailable H.264 reports unsupported and stops reconnecting', async () => {
  const b = browser({ support: async () => ({ supported: false }) }); b.hello();
  b.sockets[0].receive(frame(1)); await flush();
  assert.equal(b.context.__stats.state, 'unsupported');
  assert.ok(b.messages.some(m => m.type === 'host-viewer' && m.state === 'unsupported' && m.origin === 'https://parent.test'));
  assert.match(b.elements.get('cover').textContent, /cannot show/);
  assert.equal(b.elements.get('cover').hidden, false);
  assert.equal(b.context.UI.connected, false); assert.equal(b.sockets[0].readyState, 3);
  assert.equal(b.timers.size, 0);
});

test('tests that a missing decoder reports unsupported without keeping the socket alive', async () => {
  const b = browser({ noDecoder: true }); b.hello(); await flush();
  assert.equal(b.context.__stats.state, 'unsupported');
  assert.equal(b.sockets[0].readyState, 3); assert.equal(b.timers.size, 0);
});

test('tests that a rejected decoder configuration reports unsupported without throwing', async () => {
  const error = Object.assign(new Error('H.264 unavailable'), { name: 'NotSupportedError' });
  const b = browser({ configureError: error }); b.hello();
  assert.doesNotThrow(() => b.sockets[0].receive(frame(1))); await flush();
  assert.equal(b.context.__stats.state, 'unsupported'); assert.equal(b.timers.size, 0);
});

test('tests that reconnect backoff grows to a cap and resets after a drawn frame', async () => {
  const b = browser(), delays = [];
  for (let i = 0; i < 10; i++) { b.sockets.at(-1).close(); delays.push(b.fireTimer()); }
  assert.deepEqual(delays.slice(0, 3), [250, 500, 1000]);
  assert.equal(delays.at(-1), 10000); assert.equal(delays.at(-2), 10000);
  b.hello(); b.sockets.at(-1).receive(frame(1)); await flush();
  b.sockets.at(-1).close(); assert.equal(b.fireTimer(), 250);
});

test('tests that reconnect forgets incomplete records and requires a fresh frame before ready', async () => {
  const b = browser(); b.hello(); b.sockets[0].receive(frame(1)); await flush();
  b.sockets[0].receive(frame(2).slice(0, 6)); b.sockets[0].close(); b.fireTimer(); b.hello();
  assert.equal(b.timers.size, 0); assert.equal(b.context.__stats.state, 'connected');
  b.sockets.at(-1).receive(frame(3)); await flush();
  assert.equal(b.context.__stats.frames, 2); b.fireTimer(); assert.equal(b.context.__stats.state, 'ready');
});

test('tests that decoder output from a closed connection cannot ack or ready its replacement', async () => {
  const b = browser({ output: false }); b.hello(); b.sockets[0].receive(frame(1)); await flush();
  const stale = b.decoders[0].callbacks; b.sockets[0].close(); b.fireTimer(); b.hello();
  let closed = false;
  stale.output({ timestamp: 1, displayWidth: 640, displayHeight: 480, close() { closed = true; } });
  assert.equal(closed, true); assert.equal(b.context.__stats.frames, 0);
  assert.equal(b.sockets.at(-1).sent.length, 0); assert.equal(b.timers.size, 0);
});

test('tests that frames remain ordered while decoder support is being checked', async () => {
  let resolve;
  const b = browser({ support: () => new Promise(r => { resolve = r; }) }); b.hello();
  b.sockets[0].receive(frame(1)); b.sockets[0].receive(frame(2, false)); await flush();
  assert.equal(b.decoders.length, 0); resolve({ supported: true }); await flush();
  assert.deepEqual(b.decoders[0].decoded.map(f => f.timestamp), [1, 2]);
});

test('tests that oversized and empty records close the connection before buffering payloads', () => {
  for (const length of [0, 0xffffffff]) {
    const b = browser(); const header = new Uint8Array(4); new DataView(header.buffer).setUint32(0, length);
    assert.doesNotThrow(() => b.sockets[0].receive(header));
    assert.equal(b.sockets[0].readyState, 3); assert.equal(b.fireTimer(), 250);
  }
});

test('tests that a decoder backlog stays bounded', async () => {
  const b = browser({ support: () => new Promise(() => {}) }); b.hello();
  for (let i = 1; i <= 40; i++) b.sockets[0].receive(frame(i));
  await flush(); assert.equal(b.sockets[0].readyState, 3);
});

test('tests that paste permits 64 KiB without breaking UTF-8 at the limit', async () => {
  const text = 'a' + '€'.repeat(30000);
  const b = browser({ clipboard: async () => text }); b.hello();
  assert.equal(await b.context.slimviewer.paste(), 'sent');
  const paste = b.sockets[0].sent.at(-1);
  assert.equal(paste[4], 0x22); assert.equal(paste.length - 5, 65536);
  assert.equal(new TextDecoder('utf-8', { fatal: true }).decode(paste.subarray(5)), text.slice(0, 21846));
});

test('tests that hidden keyboard events cannot send input through a spectator connection', () => {
  const b = browser(); b.hello(undefined, true);
  const keyboard = b.elements.get('noVNC_keyboardinput');
  keyboard.listeners.get('keydown')({ key: 'a', preventDefault() {} });
  keyboard.value = 'hello'; keyboard.listeners.get('input')();
  assert.equal(b.sockets[0].sent.length, 0);
});

test('tests that pending clipboard reads cannot paste into a new owner session', async () => {
  let resolve;
  const b = browser({ clipboard: () => new Promise(r => { resolve = r; }) }); b.hello();
  const paste = b.context.slimviewer.paste(); b.sockets[0].close(); b.fireTimer(); b.hello();
  resolve('secret for the old session');
  assert.equal(await paste, 'sent'); assert.equal(b.sockets.at(-1).sent.length, 0);
});

test('tests that the page API types Unicode and presses each supported special key in order', () => {
  const b = browser(); b.hello();
  assert.equal(b.context.slimviewer.type('é日👍'), true);
  const names = ['Enter', 'Backspace', 'Tab', 'Escape', 'ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown', 'Delete', 'Home', 'End'];
  for (const name of names) assert.equal(b.context.slimviewer.key(name), true);
  assert.equal(b.sockets[0].sent[0][4], 0x23);
  assert.equal(new TextDecoder().decode(b.sockets[0].sent[0].subarray(5)), 'é日👍');
  const symbols = [0xff0d, 0xff08, 0xff09, 0xff1b, 0xff51, 0xff52, 0xff53, 0xff54, 0xffff, 0xff50, 0xff57];
  const keys = b.sockets[0].sent.filter(m => m[4] === 0x21).map(m => [new DataView(m.buffer).getUint32(5), m[9]]);
  assert.deepEqual(keys, symbols.flatMap(sym => [[sym, 1], [sym, 0]]));
});

test('tests that phone composition sends one finished commit and no preedit', () => {
  const b = browser(); b.hello();
  const kbd = b.elements.get('noVNC_keyboardinput');
  const fill = '_'.repeat(64);
  kbd.value = fill;
  kbd.listeners.get('compositionstart')({});
  for (const text of ['न', 'नम', 'नमस्ते']) {
    kbd.value = fill + text;
    kbd.listeners.get('input')({ isComposing: true });
  }
  assert.equal(b.sockets[0].sent.length, 0);
  kbd.listeners.get('compositionend')({ data: 'नमस्ते' });
  kbd.listeners.get('input')({ isComposing: false });
  assert.deepEqual(b.sockets[0].sent.map(m => m[4]), [0x23]);
  assert.equal(new TextDecoder().decode(b.sockets[0].sent[0].subarray(5)), 'नमस्ते');
});

test('tests that dictation, emoji replacement and ASCII use their proper routes', () => {
  const b = browser(); b.hello();
  const kbd = b.elements.get('noVNC_keyboardinput'), fill = '_'.repeat(64);
  kbd.value = fill + '👋🏽'; kbd.listeners.get('input')({});
  kbd.value = fill + 'é'; kbd.listeners.get('input')({ inputType: 'insertReplacementText' });
  assert.deepEqual(b.sockets[0].sent.map(m => m[4]), [0x23, 0x21, 0x21, 0x23]);
  assert.equal(new DataView(b.sockets[0].sent[1].buffer).getUint32(5), 0xff08);
  assert.equal(b.context.slimviewer.type('abc'), true);
  assert.equal(b.sockets[0].sent.at(-1)[4], 0x23);
  assert.equal(new TextDecoder().decode(b.sockets[0].sent.at(-1).subarray(5)), 'abc');
});

test('tests that non-ASCII keydown waits for the keyboard commit', () => {
  const b = browser(); b.hello();
  const kbd = b.elements.get('noVNC_keyboardinput');
  kbd.listeners.get('keydown')({ key: 'é', preventDefault() { assert.fail('must allow commit'); } });
  assert.equal(b.sockets[0].sent.length, 0);
  kbd.value = '_'.repeat(64) + 'é'; kbd.listeners.get('input')({});
  assert.deepEqual(b.sockets[0].sent.map(m => m[4]), [0x23]);
});

test('tests that a composition cannot cross reconnect or spectator boundaries', () => {
  const b = browser(); b.hello();
  const kbd = b.elements.get('noVNC_keyboardinput');
  kbd.listeners.get('compositionstart')({});
  b.sockets[0].close(); b.fireTimer(); b.hello();
  kbd.value = '_'.repeat(64) + 'secret';
  kbd.listeners.get('compositionend')({ data: 'secret' });
  kbd.listeners.get('input')({});
  assert.equal(b.sockets.at(-1).sent.length, 0);
  const spectator = browser(); spectator.hello(undefined, true);
  const field = spectator.elements.get('noVNC_keyboardinput');
  field.listeners.get('compositionstart')({}); field.value = 'नमस्ते';
  field.listeners.get('compositionend')({}); field.listeners.get('input')({});
  assert.equal(spectator.sockets[0].sent.length, 0);
});

test('tests that page API events describe connection readiness and accepted input', async () => {
  const b = browser(), events = [];
  const api = b.context.slimviewer;
  for (const name of ['state', 'ready', 'input']) api.addEventListener(name, e => events.push([name, e.detail]));
  assert.equal(api.state, 'connecting');
  b.hello(); b.sockets[0].receive(frame(1)); await flush(); b.fireTimer();
  assert.equal(api.state, 'ready');
  assert.deepEqual(events.slice(0, 3), [['state', 'connected'], ['state', 'ready'], ['ready', undefined]]);
  assert.equal(api.type('a'), true); assert.equal(api.key('Enter'), true); assert.equal(await api.paste(), 'sent');
  assert.equal(events.filter(e => e[0] === 'input').length, 3);
  b.sockets[0].close(); assert.equal(api.state, 'disconnected');
  assert.deepEqual(events.at(-1), ['state', 'disconnected']);
  assert.equal(api.type('secret'), false); assert.equal(api.key('Enter'), false); assert.equal(await api.paste(), 'blocked');
  assert.equal(events.filter(e => e[0] === 'input').length, 3);
});

test('tests that the page API rejects invalid text and key names without input', async () => {
  const b = browser(); b.hello();
  for (const text of ['', null, 7, 'a'.repeat(65537), '日'.repeat(30000)]) assert.equal(b.context.slimviewer.type(text), false);
  for (const name of ['a', 'Control', 'constructor', 'toString', null]) assert.equal(b.context.slimviewer.key(name), false);
  assert.equal(b.sockets[0].sent.length, 0);
  const spectator = browser(); spectator.hello(undefined, true);
  assert.equal(spectator.context.slimviewer.type('a'), false);
  assert.equal(spectator.context.slimviewer.key('Enter'), false);
  assert.equal(await spectator.context.slimviewer.paste(), 'blocked');
  assert.equal(spectator.sockets[0].sent.length, 0);
});

test('tests that the page paste API returns tap-again while the pill adapter returns cancelled synchronously', async () => {
  let reads = 0;
  const b = browser({ clipboard: () => { reads++; return Promise.reject(new Error('Dismissed')); } }); b.hello();
  const api = b.context.slimviewer.paste(); assert.equal(reads, 1); assert.equal(await api, 'tap-again');
  const pill = b.context.pasteFromPill(); assert.equal(reads, 2); assert.equal(await pill, 'cancelled');
  b.sockets[0].close(); assert.equal(await b.context.pasteFromPill(), 'inactive');
});

test('tests that late codec support answers cannot configure a new session', async () => {
  let resolve;
  const b = browser({ support: () => new Promise(r => { resolve = r; }) }); b.hello();
  b.sockets[0].receive(frame(1)); b.sockets[0].close(); b.fireTimer(); b.hello();
  resolve({ supported: true }); await flush();
  assert.equal(b.decoders.length, 0); assert.equal(b.context.__stats.frames, 0);
});

test('tests that an asynchronous unsupported decoder error reports fallback once', async () => {
  const b = browser({ output: false }); b.hello(); b.sockets[0].receive(frame(1)); await flush();
  b.decoders[0].callbacks.error(Object.assign(new Error('Unsupported decoder'), { name: 'NotSupportedError' }));
  assert.equal(b.context.__stats.state, 'unsupported'); assert.equal(b.timers.size, 0);
  assert.equal(b.messages.filter(m => m.state === 'unsupported').length, 1);
});

test('tests that a decoder failure requests a keyframe even if no further frame arrives', async () => {
  const b = browser({ output: false }); b.hello(); b.sockets[0].receive(frame(1)); await flush();
  b.decoders[0].callbacks.error(new Error('Corrupt frame'));
  assert.equal(b.sockets[0].sent.at(-1)[4], 0x11);
  assert.ok(b.sockets[0].sent.some(m => m[4] === 0x10));
});

test('tests that a displaced owner stays stopped while the replacement owner draws', async () => {
  const previous = browser(), replacement = browser();
  previous.hello(); previous.sockets[0].receive(frame(1)); await flush();
  previous.hello(undefined, true);
  replacement.hello(); replacement.sockets[0].receive(frame(2)); await flush();
  assert.equal(previous.context.__stats.state, 'disconnected');
  assert.equal(previous.context.UI.connected, false);
  assert.equal(previous.elements.get('cover').textContent, 'This screen was opened in another tab.');
  assert.equal(previous.sockets[0].readyState, 3); assert.equal(previous.timers.size, 0);
  assert.equal(previous.sockets.length, 1); assert.equal(replacement.context.__stats.frames, 1);
  assert.equal(replacement.context.UI.connected, true);
});

test('tests that the server owner role governs displacement even with a read-only query', () => {
  const b = browser({ viewOnlyQuery: true }); b.hello(); b.hello(undefined, true);
  assert.equal(b.elements.get('cover').textContent, 'This screen was opened in another tab.');
  assert.equal(b.timers.size, 0);
});

test('tests that other repeated greetings remain invalid rather than displacement notices', () => {
  for (const viewOnly of [false, true]) {
    const b = browser(); b.hello(undefined, viewOnly); b.hello(undefined, viewOnly);
    assert.equal(b.sockets[0].readyState, 3); assert.equal(b.fireTimer(), 250);
  }
});

test('tests that a thrown decode still acknowledges bundled frames and reaches the next keyframe', async () => {
  const b = browser({ failDecodeSeq: 2 }); b.hello(); b.sockets[0].receive(frame(1)); await flush();
  const bundled = new Uint8Array([...frame(2, false), ...frame(3, false), ...frame(4)]);
  b.sockets[0].receive(bundled); await flush();
  assert.equal(b.context.__stats.frames, 2); assert.equal(b.sockets[0].readyState, 1);
  const acks = b.sockets[0].sent.filter(m => m[4] === 0x10).map(m => new DataView(m.buffer).getUint32(5));
  assert.deepEqual(acks, [1, 2, 3, 4]);
  assert.ok(b.sockets[0].sent.some(m => m[4] === 0x11));
});
