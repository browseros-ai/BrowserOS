// Run in a disposable Linux Playwright container sharing the Neo container's network.
// NEO_CDP=http://127.0.0.1:9110 PLAYWRIGHT_MODULE=... node tests/desktop-live.mjs
// Neo: 1920x1080 Xvnc, SendCutText=0, SendPrimary=0, slimviewer on 5910/5911.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { connect } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.NEO_CDP;
assert.ok(base, 'Set NEO_CDP to a disposable Neo container');
const discovery = await (await fetch(new URL('/json/version', base))).json();
const endpoint = new URL(discovery.webSocketDebuggerUrl); endpoint.host = new URL(base).host;
const neo = await chromium.connectOverCDP(endpoint.href), browser = await chromium.launch();
const remote = await neo.contexts()[0].newPage(), remoteSession = await remote.context().newCDPSession(remote);
remote.setDefaultTimeout(10000);
const html = readFileSync(new URL('../web/slimviewer.html', import.meta.url), 'utf8');
const results = [];
function passed(name, detail) { const result = { name, ...detail, pass: true }; results.push(result); console.log(JSON.stringify(result)); }
const surface = `<!doctype html><meta charset="utf-8"><style>
body { margin:0; font:20px sans-serif; width:3500px; height:7000px; background:linear-gradient(white,#aab); }
textarea { margin:30px; width:900px; height:180px; font:22px monospace; }
#words { margin:30px; width:1200px; font:24px monospace; }
#area { margin:30px; width:1200px; height:500px; background:#dde; }
</style><textarea id="field" autofocus></textarea><div id="words">Drag select this sentence across the remote page.</div>
<div id="area">Scroll vertically and horizontally here. Right and middle clicks are recorded.</div>
<script>window.events=[];for(const type of ['keydown','keyup','mousedown','mouseup','dblclick','contextmenu','wheel'])
addEventListener(type,e=>events.push({type,key:e.key,button:e.button,detail:e.detail,dx:e.deltaX,dy:e.deltaY,mode:e.deltaMode}),true);</script>`;
let viewerContext, viewer;
try {
  await remote.goto('data:text/html;charset=utf-8,' + encodeURIComponent(surface)); await remote.bringToFront();
  const { targetInfo } = await remoteSession.send('Target.getTargetInfo');
  const { windowId } = await remoteSession.send('Browser.getWindowForTarget', { targetId: targetInfo.targetId });
  await remoteSession.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'maximized' } });
  await remote.waitForFunction(() => outerWidth === 1920 && outerHeight >= 1000);
  viewerContext = await browser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ['clipboard-read', 'clipboard-write'] });
  await viewerContext.route('https://desktop.test/**', route => route.fulfill({ contentType: 'text/html', body: html }));
  await viewerContext.route('https://local.test/**', route => route.fulfill({ contentType: 'text/html', body: '<textarea id="local" autofocus style="width:900px;height:400px"></textarea>' }));
  await viewerContext.routeWebSocket('**/slim', socket => {
    const peer = connect({ host: '127.0.0.1', port: 5910 });
    peer.on('data', data => socket.send(data));
    peer.on('error', () => socket.close()); peer.on('close', () => socket.close());
    socket.onMessage(data => peer.write(data)); socket.onClose(() => peer.destroy());
  });
  viewer = await viewerContext.newPage(); const errors = [];
  viewer.setDefaultTimeout(10000);
  viewer.on('pageerror', error => errors.push(error.message));
  await viewer.goto('https://desktop.test/');
  await viewer.locator('#cover').waitFor({ state: 'hidden', timeout: 20000 });
  assert.deepEqual(await viewer.evaluate(() => [__stats.w, __stats.h]), [1920, 1080]);
  assert.equal(await viewer.locator('#controls').isVisible(), false);
  const canvas = await viewer.locator('#screen').boundingBox();
  const position = async (selector, fx = 0.5, fy = 0.5) => {
    const point = await remote.locator(selector).evaluate((el, [fx, fy]) => {
      const r = el.getBoundingClientRect(), border = (outerWidth - innerWidth) / 2;
      return { x: screenX + border + r.x + r.width * fx, y: screenY + outerHeight - innerHeight - border + r.y + r.height * fy };
    }, [fx, fy]);
    return [canvas.x + point.x * canvas.width / 1920, canvas.y + point.y * canvas.height / 1080];
  };
  const click = async selector => viewer.mouse.click(...await position(selector));
  const valueIs = text => remote.waitForFunction(text => document.getElementById('field').value === text, text);
  const clipboardIs = text => viewer.waitForFunction(async text => await navigator.clipboard.readText() === text, text);
  const selected = () => remote.locator('#field').evaluate(el => el.value.slice(el.selectionStart, el.selectionEnd));
  await click('#field');
  const paragraph = 'A desktop viewer should feel familiar.\nCopy this sentence from Neo.';
  await viewer.keyboard.type(paragraph, { delay: 3 }); await valueIs(paragraph);
  await viewer.keyboard.insertText(' 日本 👩🏽‍💻'); await valueIs(paragraph + ' 日本 👩🏽‍💻');
  passed('physical paragraph and Unicode TYPE', { characters: (paragraph + ' 日本 👩🏽‍💻').length });
  await viewer.keyboard.press('Home'); await viewer.keyboard.press('Shift+End');
  await remote.waitForFunction(() => { const f = document.getElementById('field'); return f.value.slice(f.selectionStart, f.selectionEnd) === 'Copy this sentence from Neo. 日本 👩🏽‍💻'; });
  const sentence = await selected(); assert.equal(sentence, 'Copy this sentence from Neo. 日本 👩🏽‍💻');
  await viewer.keyboard.press('Control+c'); await clipboardIs(sentence);
  const local = await viewerContext.newPage(); await local.goto('https://local.test/');
  await local.locator('#local').click(); await local.keyboard.press('Control+v');
  await local.waitForFunction(text => document.getElementById('local').value === text, sentence);
  passed('Neo copy pasted into a local textarea', { bytes: Buffer.byteLength(sentence) });
  await viewer.bringToFront();
  await viewer.keyboard.press('Control+x');
  await valueIs('A desktop viewer should feel familiar.\n'); await clipboardIs(sentence);
  await viewer.keyboard.press('Control+z'); await valueIs(paragraph + ' 日本 👩🏽‍💻');
  passed('cut and undo', {});
  await viewer.evaluate(() => navigator.clipboard.writeText('Pasted from desktop: café 日本 👋'));
  await viewer.keyboard.press('Control+a'); await viewer.keyboard.press('Control+v');
  await valueIs('Pasted from desktop: café 日本 👋'); passed('desktop Ctrl+V paste', {});
  // Preserve a real browser context menu: invoke Copy with the native menu's keyboard mnemonic.
  await viewer.keyboard.press('Control+a');
  await remote.waitForFunction(() => { const f = document.getElementById('field'); return f.selectionStart === 0 && f.selectionEnd === f.value.length; });
  await viewer.evaluate(() => navigator.clipboard.writeText('before context-menu Copy'));
  const beforeMenu = await viewer.evaluate(() => __stats.frames);
  await viewer.mouse.click(...await position('#field', 0.12, 0.08), { button: 'right' });
  await remote.waitForFunction(() => events.some(e => e.type === 'contextmenu'));
  await viewer.waitForFunction(before => __stats.frames >= before + 2, beforeMenu);
  await viewer.screenshot({ path: '/tmp/desktop-menu.png' });
  await viewer.keyboard.press('c');
  await clipboardIs('Pasted from desktop: café 日本 👋');
  await valueIs('Pasted from desktop: café 日本 👋');
  passed('native right-click menu Copy reaches local clipboard', {});
  await click('#area'); await viewer.mouse.click(...await position('#area'), { button: 'middle' });
  await remote.waitForFunction(() => events.some(e => e.type === 'mouseup' && e.button === 1));
  await viewer.mouse.dblclick(...await position('#words', 0.06), { delay: 80 });
  await remote.waitForFunction(() => events.some(e => e.type === 'dblclick'));
  await viewer.mouse.move(...await position('#words', 0.001)); await viewer.mouse.down();
  await viewer.mouse.move(...await position('#words', 0.43), { steps: 12 }); await viewer.mouse.up();
  const selection = await remote.evaluate(() => getSelection().toString());
  assert.ok(selection.includes('select this sentence'), selection);
  await viewer.keyboard.press('Control+c'); await clipboardIs(selection);
  passed('middle click, double click, drag-select and document copy', { selection });
  await viewer.mouse.move(...await position('#area'));
  await viewer.mouse.wheel(0, 333);
  await remote.waitForFunction(() => scrollY > 0);
  await viewer.mouse.wheel(160, 0);
  await remote.waitForFunction(() => scrollX > 0);
  const scrolling = await remote.evaluate(() => ({ x: scrollX, y: scrollY, wheels: events.filter(e => e.type === 'wheel') }));
  assert.ok(scrolling.wheels.some(e => e.dy > 0 && e.mode === 0));
  assert.ok(scrolling.wheels.some(e => e.dx > 0 && e.mode === 0));
  passed('pixel wheel on both axes', scrolling);
  await viewer.screenshot({ path: process.env.DESKTOP_SCREENSHOT || '/tmp/desktop-live.png' });
  assert.deepEqual(errors, []);
  passed('real H.264 viewer', { neo: await neo.version(), viewer: await browser.version(), viewport: { width: 1280, height: 800 }, framebuffer: [1920, 1080], frames: await viewer.evaluate(() => __stats.frames) });
} catch (error) {
  await viewer?.screenshot({ path: '/tmp/desktop-failure.png' });
  console.error('Remote state:', await remote.evaluate(() => ({ focus: document.activeElement.id, text: document.getElementById('field').value, selection: [document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd], events: events.slice(-30) })));
  throw error;
} finally {
  await viewerContext?.close(); await remote.close(); await browser.close(); await neo.close();
}
