// PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node --test tests/agent-chromium.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { before, after, test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const source = await readFile(new URL('../src/rfb/cdp/agent.js', import.meta.url), 'utf8');
let browser, server, childServer, origin, childOrigin;
let transaction = 0;
// Assertions and browser errors deliberately omit page contents, including on failure.
function check(condition, message = 'expected browser behavior') { assert.ok(condition, message); }
async function startServer() {
  const server = createServer(async (req, res) => {
    const name = req.url?.split('?')[0].slice(1) || 'fields.html';
    if (name === 'startup.html') {
      res.writeHead(200, {'Content-Type': 'text/html; charset=utf-8'});
      res.write('<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><title>Startup</title>');
      setTimeout(() => res.end('<body><input style="position:absolute;left:20px;top:20px;width:120px;height:30px"></body>'), 30);
      return;
    }
    if (!/^[a-z-]+\.html$/.test(name)) { res.writeHead(404).end(); return; }
    try {
      let html = await readFile(new URL(`./agent-fixtures/${name}`, import.meta.url), 'utf8');
      html = html.replaceAll('__CHILD_ORIGIN__', childOrigin || '');
      res.writeHead(200, {'Content-Type': 'text/html; charset=utf-8'}).end(html);
    } catch { res.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return server;
}
before(async () => {
  childServer = await startServer();
  childOrigin = `http://localhost:${childServer.address().port}`;
  server = await startServer();
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({headless: true, args: ['--site-per-process']});
});
after(async () => {
  await browser?.close();
  await Promise.all([server, childServer].filter(Boolean).map(s => new Promise(resolve => s.close(resolve))));
});
async function install(context, frame) {
  const cdp = await context.newCDPSession(frame);
  await cdp.send('Page.enable');
  await cdp.send('Runtime.enable');
  const reports = [], reportTimes = [];
  cdp.on('Runtime.bindingCalled', event => {
    if (event.name === 'slimviewerAgent') {
      reports.push(JSON.parse(event.payload));
      reportTimes.push(performance.now());
    }
  });
  await cdp.send('Runtime.addBinding', {name: 'slimviewerAgent', executionContextName: 'slimviewer-input'});
  await cdp.send('Page.addScriptToEvaluateOnNewDocument', {source, worldName: 'slimviewer-input'});
  const { frameTree } = await cdp.send('Page.getFrameTree');
  const { executionContextId } = await cdp.send('Page.createIsolatedWorld', {
    frameId: frameTree.frame.id, worldName: 'slimviewer-input', grantUniveralAccess: false,
  });
  async function evaluate(expression) {
    const result = await cdp.send('Runtime.evaluate', {expression, contextId: executionContextId, returnByValue: true, awaitPromise: true});
    check(!result.exceptionDetails, 'isolated agent call must succeed');
    return result.result.value;
  }
  await evaluate(source);
  const call = (method, argument) => evaluate(`__sv.${method}(${JSON.stringify(argument)})`);
  return {cdp, reports, reportTimes, evaluate, call};
}
async function fixture(run, name = 'fields.html') {
  const context = await browser.newContext({viewport: {width: 900, height: 900}, ignoreHTTPSErrors: true});
  try {
    const page = await context.newPage();
    await page.goto(`${origin}/${name}`);
    const agent = await install(context, page);
    await run({context, page, ...agent});
  } catch (error) {
    if (error instanceof assert.AssertionError) throw error;
    throw new Error('Chromium fixture operation failed');
  } finally { await context.close(); }
}
async function paste(agent, text = 'Z', options = {}) {
  const id = ++transaction;
  const prepared = await agent.call('prepare', {id, text, refocus: false, retry: false, ...options});
  let inserts = 0;
  if (prepared.state === 'ready') { await agent.cdp.send('Input.insertText', {text}); inserts++; }
  const evidence = ['ready', 'handled'].includes(prepared.state)
    ? await agent.call('verify', {id, windowMs: prepared.state === 'handled' ? 300 : 50}) : null;
  return {prepared, evidence, inserts, id};
}
function evidenceShape(evidence) {
  assert.deepEqual(Object.keys(evidence).sort(), ['beforeInput', 'changed', 'focusMoved', 'gone', 'input'].sort());
  for (const key of ['changed', 'focusMoved', 'gone']) assert.equal(typeof evidence[key], 'boolean');
  for (const key of ['beforeInput', 'input']) check(Number.isInteger(evidence[key]) && evidence[key] >= 0);
}

test('tests that a plain field uses isolated prepare then trusted insertion and metadata evidence', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await agent.page.locator('#plain').evaluate(el => el.setSelectionRange(2, 4));
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'ready');
    assert.equal(result.inserts, 1);
    evidenceShape(result.evidence);
    check(result.evidence.changed && result.evidence.beforeInput > 0 && result.evidence.input > 0);
    check(await agent.page.evaluate(() => document.querySelector('#plain').value === 'abZef'));
    check(await agent.page.evaluate(() => probe.pastes === 1 && probe.sawText && probe.untrusted && probe.composed && probe.inputTrusted));
  });
});

for (const selector of ['#textarea', '#password', '#editable', '#shadow-input']) {
  test(`tests that ${selector.slice(1)} receives one insertion through the deep active field`, async () => {
    await fixture(async agent => {
      await agent.page.locator(selector).click();
      await agent.page.locator(selector).evaluate(el => {
        if ('setSelectionRange' in el) el.setSelectionRange(2, 4);
        else { const range = new Range(); range.setStart(el.firstChild, 2); range.setEnd(el.firstChild, 4); getSelection().removeAllRanges(); getSelection().addRange(range); }
      });
      const result = await paste(agent);
      assert.equal(result.prepared.state, 'ready');
      check(result.evidence.changed && result.evidence.input > 0 && !result.evidence.focusMoved && !result.evidence.gone);
      check(await agent.page.locator(selector).evaluate(el => (el.value ?? el.textContent) === 'abZef'));
      check(await agent.page.evaluate(() => probe.pastes === 1 && probe.sawText && probe.composed));
    });
  });
}

test('tests that isolated paste reaches the main-world OTP distributor without any insertion', async () => {
  await fixture(async agent => {
    await agent.page.locator('#otp0').click();
    const result = await paste(agent, '123456');
    assert.equal(result.prepared.state, 'handled');
    assert.equal(result.inserts, 0);
    check(result.evidence.changed);
    assert.equal(result.evidence.input, 0);
    check(await agent.page.evaluate(() => [...document.querySelectorAll('#otp input')].map(el => el.value).join('') === '123456'));
  });
});

test('tests that verification detects a sibling-only OTP update with no input events', async () => {
  await fixture(async agent => {
    await agent.page.locator('#otp0').evaluate(el => el.addEventListener('paste', event => {
      event.stopImmediatePropagation(); event.preventDefault(); el.nextElementSibling.value = 'Z';
    }, true));
    await agent.page.locator('#otp0').click();
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'handled');
    check(result.evidence.changed);
    assert.equal(result.evidence.input, 0);
    check(await agent.page.evaluate(() => document.querySelector('#otp0').value === '' && document.querySelector('#otp0').nextElementSibling.value === 'Z'));
  });
});

test('tests that a prevented paste stays unchanged and is never followed by insertion', async () => {
  await fixture(async agent => {
    await agent.page.locator('#blocker').click();
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'handled');
    assert.equal(result.inserts, 0);
    check(!result.evidence.changed && result.evidence.input === 0 && result.evidence.beforeInput === 0);
    check(await agent.page.evaluate(() => document.querySelector('#blocker').value === 'abcdef'));
  });
});

test('tests that handled verification waits for a delayed application update', async () => {
  await fixture(async agent => {
    await agent.page.locator('#delayed').click();
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'handled');
    check(result.evidence.changed);
    check(await agent.page.evaluate(() => document.querySelector('#delayed').value === 'abcdefZ'));
  });
});

test('tests that ProseMirror consumes the isolated-world paste exactly once', async () => {
  await fixture(async agent => {
    check(await agent.page.evaluate(() => loadProseMirror()), 'pinned ProseMirror CDN fixture must load');
    await agent.page.locator('#prosemirror').click();
    await agent.page.evaluate(() => selectProseMirrorText(3, 5));
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'handled');
    assert.equal(result.inserts, 0);
    check(result.evidence.changed);
    check(await agent.page.evaluate(() => pmview.state.doc.textContent === 'abZef'));
  });
});

for (const selector of ['#plain', '#editable']) {
  test(`tests that refocus restores the ${selector.slice(1)} caret without scrolling`, async () => {
    await fixture(async agent => {
      await agent.page.locator(selector).click();
      await agent.page.locator(selector).evaluate(el => {
        if ('setSelectionRange' in el) el.setSelectionRange(2, 4);
        else { const range = new Range(); range.setStart(el.firstChild, 2); range.setEnd(el.firstChild, 4); getSelection().removeAllRanges(); getSelection().addRange(range); }
        el.blur();
        if (el.isContentEditable) getSelection().removeAllRanges();
        scrollTo(0, 1100);
      });
      const before = await agent.page.evaluate(() => scrollY);
      const id = ++transaction;
      const prepared = await agent.call('prepare', {id, text: 'Z', refocus: true, retry: false});
      assert.equal(prepared.state, 'ready');
      assert.equal(await agent.page.evaluate(() => scrollY), before);
      await agent.cdp.send('Input.insertText', {text: 'Z'});
      const evidence = await agent.call('verify', {id, windowMs: 50});
      check(evidence.changed);
      check(await agent.page.locator(selector).evaluate(el => (el.value ?? el.textContent) === 'abZef'));
    });
  });
}

test('tests that retry refocuses the original target and skips a second synthetic paste', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    const id = ++transaction;
    assert.equal((await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false})).state, 'ready');
    const first = await agent.call('verify', {id, windowMs: 50});
    check(!first.changed && first.input === 0 && first.beforeInput === 0);
    await agent.page.locator('#plain').evaluate(el => el.blur());
    assert.equal((await agent.call('prepare', {id, text: 'Z', refocus: true, retry: true})).state, 'ready');
    await agent.cdp.send('Input.insertText', {text: 'Z'});
    const evidence = await agent.call('verify', {id, windowMs: 50});
    check(evidence.changed);
    check(await agent.page.evaluate(() => probe.pastes === 1));
  });
});

test('tests that noneditable trusted taps replace old field memory and synthetic taps never arm refocus', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await agent.page.locator('#elsewhere').click();
    let result = await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false});
    assert.equal(result.state, 'notfocused');
    await agent.page.locator('#plain').dispatchEvent('pointerdown', {bubbles: true, pointerType: 'mouse'});
    result = await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false});
    assert.equal(result.state, 'notfocused');
    const taps = agent.reports.filter(r => r.k === 'tap');
    check(taps.length === 2 && taps[0].editable && !taps[1].editable);
  });
});

test('tests that removed and newly readonly tapped fields cannot be refocused', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await agent.page.locator('#plain').evaluate(el => { el.blur(); el.remove(); });
    assert.equal((await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false})).state, 'notfocused');
    await agent.page.locator('#textarea').click();
    await agent.page.locator('#textarea').evaluate(el => { el.blur(); el.readOnly = true; });
    assert.equal((await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false})).state, 'notfocused');
  });
});

test('tests that no-field paste still reaches application handlers and reports whether it was consumed', async () => {
  await fixture(async agent => {
    await agent.page.locator('#elsewhere').click();
    let result = await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: false, retry: false});
    assert.deepEqual(result, {state: 'nofield', handled: false});
    await agent.page.evaluate(() => document.body.addEventListener('paste', event => event.preventDefault()));
    result = await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: false, retry: false});
    assert.deepEqual(result, {state: 'nofield', handled: true});
  });
});

test('tests that evidence observes focus movement, removed targets, and unrelated document input', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    const id = ++transaction;
    await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
    await agent.page.evaluate(() => {
      document.querySelector('#textarea').focus();
      document.querySelector('#textarea').dispatchEvent(new InputEvent('beforeinput', {bubbles: true}));
      document.querySelector('#textarea').dispatchEvent(new InputEvent('input', {bubbles: true}));
      document.querySelector('#plain').remove();
    });
    const evidence = await agent.call('verify', {id, windowMs: 50});
    check(evidence.focusMoved && evidence.gone && evidence.beforeInput > 0 && evidence.input > 0);
  });
});

async function eventually(check, message, timeout = 2500) {
  const end = Date.now() + timeout;
  do { if (await check()) return; await delay(20); } while (Date.now() < end);
  assert.fail(message);
}
const boxReports = agent => agent.reports.filter(r => r.k === 'boxes');
const lastBoxes = agent => boxReports(agent).at(-1);
const hasBox = (report, wanted) => report?.boxes.some(box => box.every((n, i) => Math.abs(n - wanted[i]) < 1));
function reportShape(report) {
  assert.deepEqual(Object.keys(report).sort(), ['boxes', 'doc', 'focused', 'k', 'vp']);
  check(Number.isSafeInteger(report.doc) && report.doc > 0);
  assert.deepEqual(Object.keys(report.vp).sort(), ['dpr', 'inner', 'screen', 'vv', 'vv_scale', 'zoom']);
  assert.equal(report.vp.vv.length, 4); assert.equal(report.vp.inner.length, 2); assert.equal(report.vp.screen.length, 2);
  check(report.focused === null || Number.isInteger(report.focused) && report.focused >= 0 && report.focused < report.boxes.length);
  for (const box of report.boxes) check(box.length === 4 && box.every(Number.isFinite) && box[2] > 0 && box[3] > 0);
}

test('tests that field boxes filter noneditable, hidden, covered and offscreen fields while clipping visible fields', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial box report must arrive');
    const report = lastBoxes(agent); reportShape(report);
    for (const rect of [[20, 20, 120, 30], [20, 70, 120, 30], [20, 120, 120, 30], [20, 170, 120, 30], [0, 220, 90, 30], [20, 270, 120, 30], [20, 320, 120, 30]]) check(hasBox(report, rect), 'eligible field geometry must be present');
    for (const rect of [[200, 20, 120, 30], [200, 70, 120, 30], [200, 120, 120, 30], [200, 170, 120, 30], [1000, 20, 120, 30]]) check(!hasBox(report, rect), 'ineligible field geometry must be absent');
    await agent.page.locator('#plain').focus();
    await eventually(() => lastBoxes(agent)?.focused !== null, 'focus index must refresh');
    check(hasBox({boxes: [lastBoxes(agent).boxes[lastBoxes(agent).focused]]}, [20, 20, 120, 30]));
  }, 'geometry.html');
});

test('tests that geometry refreshes after nested scroll, resize, DOM insertion and animation completion', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    let previous = boxReports(agent).length;
    await agent.page.evaluate(() => { document.querySelector('#nested').scrollTop = 15; });
    await eventually(() => boxReports(agent).length > previous, 'capture scroll must refresh geometry');
    previous = boxReports(agent).length;
    await agent.page.evaluate(() => { const input = document.createElement('input'); input.id = 'new-field'; input.style.cssText = 'left:400px;top:20px'; document.body.append(input); });
    await eventually(() => hasBox(lastBoxes(agent), [400, 20, 120, 30]), 'subtree insertion must refresh geometry');
    await agent.page.evaluate(() => { const el = document.querySelector('#new-field'); el.style.left = '450px'; el.dispatchEvent(new Event('transitionend', {bubbles: true})); });
    await eventually(() => hasBox(lastBoxes(agent), [450, 20, 120, 30]), 'transition completion must refresh geometry');
    await agent.page.evaluate(() => { const el = document.querySelector('#new-field'); el.style.left = '500px'; el.dispatchEvent(new Event('animationend', {bubbles: true})); });
    await eventually(() => hasBox(lastBoxes(agent), [500, 20, 120, 30]), 'animation completion must refresh geometry');
    await agent.page.setViewportSize({width: 550, height: 900});
    await eventually(() => lastBoxes(agent)?.vp.inner[0] === 550 && hasBox(lastBoxes(agent), [500, 20, 50, 30]), 'resize must refresh viewport and clip boxes');
  }, 'geometry.html');
});

test('tests that keepalive finds silent layout changes and suppresses unchanged box reports', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    const count = boxReports(agent).length;
    await delay(1150);
    assert.equal(boxReports(agent).length, count);
    await agent.page.evaluate(() => { document.querySelector('#plain').style.left = '55px'; });
    await eventually(() => hasBox(lastBoxes(agent), [55, 20, 120, 30]), 'visible-document keepalive must find silent layout change');
  }, 'geometry.html');
});

test('tests that geometry limits output to 128 fields and scanning to the first 512 candidates', async () => {
  await fixture(async agent => {
    await agent.page.evaluate(() => {
      document.body.replaceChildren();
      for (let i = 0; i < 140; i++) { const el = document.createElement('input'); el.style.cssText = `left:${i % 20 * 8}px;top:${Math.floor(i / 20) * 8}px;width:5px;height:5px`; document.body.append(el); }
    });
    await eventually(() => lastBoxes(agent)?.boxes.length === 128, 'box cap must be exactly 128 eligible fields');
    await agent.page.evaluate(() => {
      document.body.replaceChildren();
      for (let i = 0; i < 512; i++) { const el = document.createElement('input'); el.hidden = true; document.body.append(el); }
      const late = document.createElement('input'); late.style.cssText = 'left:400px;top:400px'; document.body.append(late);
    });
    await eventually(() => lastBoxes(agent)?.boxes.length === 0, 'candidates beyond 512 must never be scanned');
  }, 'geometry.html');
});

test('tests that continuous geometry triggers obey the maximum wait', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    const count = boxReports(agent).length;
    await agent.page.evaluate(() => {
      let left = 20;
      window.geometryTimer = setInterval(() => { document.querySelector('#plain').style.left = `${++left}px`; dispatchEvent(new Event('resize')); }, 30);
    });
    try { await eventually(() => boxReports(agent).length > count, 'continuous events must report within 300 ms plus scheduling margin', 650); }
    finally { await agent.page.evaluate(() => clearInterval(window.geometryTimer)); }
  }, 'geometry.html');
});

test('tests that the first geometry change after a quiet spell reports without the throttle delay', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    await delay(400);
    const count = boxReports(agent).length;
    const started = Date.now();
    await agent.page.evaluate(() => { document.querySelector('#plain').style.left = '77px'; dispatchEvent(new Event('resize')); });
    await eventually(() => boxReports(agent).length > count, 'a quiet page must report its first change before the 120 ms throttle', 1000);
    // The old trailing debounce waited 120 ms; the leading edge reports on the next task.
    assert.ok(Date.now() - started < 100, `first change reported after ${Date.now() - started} ms`);
  }, 'geometry.html');
});

test('tests that a new document reports its fields before their first painted frames', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    const previous = lastBoxes(agent).doc;
    await agent.page.goto(`${origin}/startup.html`);
    await agent.page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await agent.cdp.send('Runtime.evaluate', {expression: '0'});
    const report = lastBoxes(agent);
    check(report.doc !== previous && report.boxes.length === 1,
      'document-start empty scans must not delay the first visible fields');
  });
});

test('tests that the first scroll after a document loads updates boxes before its painted frames', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    const wanted = await agent.page.evaluate(async () => {
      document.querySelector('#nested').scrollTop = 15;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const r = document.querySelector('#nested-field').getBoundingClientRect();
      return [r.left, r.top, r.width, r.height];
    });
    await agent.cdp.send('Runtime.evaluate', {expression: '0'});
    check(hasBox(lastBoxes(agent), wanted), 'the first scroll must not wait behind the initial scan');
  }, 'geometry.html');
});

test('tests that two completed scrolls publish the final position before its painted frames', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    for (const top of [5, 15]) {
      const wanted = await agent.page.evaluate(async top => {
        document.querySelector('#nested').scrollTop = top;
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const r = document.querySelector('#nested-field').getBoundingClientRect();
        return [r.left, r.top, r.width, r.height];
      }, top);
      await eventually(() => hasBox(lastBoxes(agent), wanted), 'a completed scroll must report its final position within the throttle interval');
    }
  }, 'geometry.html');
});

test('tests that repeated scroll endings refresh boxes at most 13 times per 500 ms', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    const count = boxReports(agent).length;
    const reportStart = agent.reports.length;
    await agent.page.evaluate(async () => {
      const started = performance.now();
      for (let i = 0; i < 30; i++) {
        document.dispatchEvent(new Event('scrollend'));
        const remaining = started + (i + 1) * 500 / 30 - performance.now();
        if (remaining > 0) await new Promise(resolve => setTimeout(resolve, remaining));
      }
    });
    await agent.cdp.send('Runtime.evaluate', {expression: '0'});
    const reports = boxReports(agent).length - count;
    const times = agent.reports.flatMap((report, index) => index >= reportStart && report.k === 'boxes' ? [agent.reportTimes[index]] : []);
    const shortest = Math.min(...times.slice(1).map((at, i) => at - times[i]));
    assert.ok(reports <= 13, `scroll-end scans must respect the 40 ms minimum interval (received ${reports})`);
    assert.ok(shortest >= 40, `box reports must be at least 40 ms apart (shortest ${shortest})`);
  });
});

test('tests that input reports contain metadata only and are throttled to one per 50 milliseconds', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await agent.page.evaluate(() => {
      for (let i = 0; i < 30; i++) document.querySelector('#plain').dispatchEvent(new InputEvent('input', {bubbles: true}));
    });
    await agent.cdp.send('Runtime.evaluate', {expression: '0'});
    let reports = agent.reports.filter(r => r.k === 'input');
    assert.equal(reports.length, 1);
    await delay(70);
    const previous = agent.reports.filter(r => r.k === 'input').length;
    await agent.page.locator('#plain').dispatchEvent('input');
    await eventually(() => agent.reports.filter(r => r.k === 'input').length > previous, 'later input metadata must be sent');
    reports = agent.reports.filter(r => r.k === 'input');
    for (let i = 1; i < reports.length; i++) check(reports[i].at - reports[i - 1].at >= 50);
    const times = agent.reportTimes.filter((_, index) => agent.reports[index].k === 'input');
    for (let i = 1; i < times.length; i++) check(times[i] - times[i - 1] >= 40, 'input reports must be spaced by 50 ms within CDP scheduling tolerance');
    for (const report of agent.reports) {
      if (report.k === 'boxes') reportShape(report);
      else assert.deepEqual(Object.keys(report).sort(), ({tap: ['at', 'editable', 'k'], focus: ['at', 'editable', 'inside', 'k'], input: ['at', 'k']})[report.k]);
    }
  });
});

test('tests that stop removes event observers, timers and remembered refocus targets', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    await agent.call('stop');
    const count = agent.reports.length;
    await agent.page.locator('#textarea').click();
    await agent.page.evaluate(() => { document.querySelector('#textarea').dispatchEvent(new InputEvent('input', {bubbles: true})); document.body.append(document.createElement('input')); dispatchEvent(new Event('resize')); });
    await delay(1200);
    assert.equal(agent.reports.length, count);
  });
});

test('tests that cross-site iframe preparation and page-session insertion share the real focused target', async () => {
  await fixture(async parent => {
    const child = parent.page.frames().find(frame => frame !== parent.page.mainFrame());
    check(child, 'cross-site child fixture must exist');
    await child.waitForSelector('#plain');
    const agent = await install(parent.context, child);
    const {targetInfo} = await agent.cdp.send('Target.getTargetInfo');
    assert.equal(targetInfo.type, 'iframe', 'different sites must produce a real OOPIF');
    await child.locator('#plain').click();
    await child.locator('#plain').evaluate(el => el.setSelectionRange(2, 4));
    const id = ++transaction;
    const prepared = await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
    assert.equal(prepared.state, 'ready');
    await parent.cdp.send('Input.insertText', {text: 'Z'});
    const evidence = await agent.call('verify', {id, windowMs: 50});
    check(evidence.changed && evidence.input > 0);
    check(!parent.reports.some(report => report.k === 'input'), 'child input must stay scoped to its document');
    check(await child.evaluate(() => document.querySelector('#plain').value === 'abZef'));
    const rect = await parent.evaluate('__sv.frameRect(document.querySelector("iframe"))');
    assert.deepEqual(rect, [47, 67, 450, 650]);
    check(parent.reports.some(r => r.k === 'focus' && !r.editable && !r.inside), 'parent iframe focus must not claim a field');
    await eventually(() => lastBoxes(agent), 'child must report local field geometry');
    reportShape(lastBoxes(agent));
  }, 'frames.html');
});

test('tests that a cancelled beforeinput leaves the field unchanged with refusal evidence', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').evaluate(el => el.addEventListener('beforeinput', event => event.preventDefault()));
    await agent.page.locator('#plain').click();
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'ready');
    check(!result.evidence.changed && result.evidence.beforeInput > 0 && result.evidence.input === 0);
    check(await agent.page.evaluate(() => document.querySelector('#plain').value === 'abcdef'));
  });
});

test('tests that maxlength truncation still counts as landing without a second insertion', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').evaluate(el => { el.value = ''; el.maxLength = 2; });
    await agent.page.locator('#plain').click();
    const result = await paste(agent, 'abcdef');
    assert.equal(result.prepared.state, 'ready');
    assert.equal(result.inserts, 1);
    check(result.evidence.changed && result.evidence.input > 0);
    check(await agent.page.evaluate(() => document.querySelector('#plain').value === 'ab'));
  });
});

test('tests that a tap older than ten minutes cannot restore an unfocused field', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    await agent.page.locator('#plain').evaluate(el => el.blur());
    await agent.evaluate('Date.now = (() => { const now = Date.now(); return () => now + 11 * 60 * 1000; })()');
    const result = await agent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false});
    assert.equal(result.state, 'notfocused');
  });
});

test('tests that iframe content geometry accounts for borders, CSS scale and parent scrolling', async () => {
  await fixture(async agent => {
    assert.deepEqual(await agent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [47, 67, 450, 650]);
    await agent.page.locator('iframe').evaluate(el => { el.style.transformOrigin = 'top left'; el.style.transform = 'scale(0.5)'; });
    assert.deepEqual(await agent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [43.5, 63.5, 225, 325]);
    await agent.page.locator('iframe').evaluate(el => { el.style.transform = ''; el.style.top = '600px'; document.body.style.height = '1800px'; scrollTo(0, 300); });
    assert.deepEqual(await agent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [47, 307, 450, 650]);
  }, 'frames.html');
});

test('tests that a completed parent scroll refreshes iframe geometry without local fields', async () => {
  await fixture(async agent => {
    await agent.page.evaluate(() => {
      document.querySelector('#parent-field').remove();
      document.body.style.height = '1800px';
      document.querySelector('iframe').style.top = '600px';
    });
    await eventually(() => lastBoxes(agent)?.boxes.length === 0, 'parent must have no local fields');
    const previous = boxReports(agent).length;
    await agent.page.evaluate(async () => {
      scrollTo(0, 300);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    assert.deepEqual(await agent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [47, 307, 450, 650]);
    await eventually(() => boxReports(agent).length > previous, 'the server must remeasure the moved iframe within the throttle interval even when parent boxes are unchanged');
  }, 'frames.html');
});

test('tests that page-world scripts cannot access the agent or its reporting binding', async () => {
  await fixture(async agent => {
    check(await agent.page.evaluate(() => typeof globalThis.__sv === 'undefined' && typeof globalThis.slimviewerAgent === 'undefined'));
    check(await agent.evaluate('typeof globalThis.__sv.prepare === "function" && typeof globalThis.slimviewerAgent === "function"'));
  });
});

test('tests that textarea insertion preserves Unicode, multiline and long text without reporting contents', async () => {
  await fixture(async agent => {
    for (const text of ['café', '👍🏽', 'नमस्ते', '你好', 'x'.repeat(2000), 'one\ntwo\nthree', '   ', 'https://example.invalid/path']) {
      await agent.page.locator('#textarea').evaluate(el => { el.value = ''; });
      await agent.page.locator('#textarea').click();
      const before = await agent.page.evaluate(() => [probe.beforeInputs, probe.inputs]);
      const result = await paste(agent, text);
      const after = await agent.page.evaluate(() => [probe.beforeInputs, probe.inputs]);
      assert.equal(result.prepared.state, 'ready');
      check(result.evidence.changed);
      assert.equal(result.evidence.beforeInput, after[0] - before[0]);
      assert.equal(result.evidence.input, after[1] - before[1]);
      check(result.evidence.input > 0);
      check(await agent.page.locator('#textarea').evaluate((el, expected) => el.value === expected, text), 'textarea must preserve the inserted text');
    }
  });
});

test('tests that visual viewport scaling refreshes viewport metadata and clips local field boxes', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'initial geometry required');
    await agent.page.evaluate(() => { document.querySelector('#plain').style.left = '400px'; });
    await agent.cdp.send('Emulation.setPageScaleFactor', {pageScaleFactor: 2});
    await eventually(() => lastBoxes(agent)?.vp.vv_scale === 2 && Math.abs(lastBoxes(agent).vp.vv[2] - 450) < 1, 'visual viewport resize must refresh scale and visible width');
    check(hasBox(lastBoxes(agent), [400, 20, 50, 30]), 'boxes must clip to the narrower visual viewport');
  }, 'geometry.html');
});

test('tests that parent refocus cannot steal the caret from an active cross-site child field', async () => {
  await fixture(async parent => {
    const child = parent.page.frames().find(frame => frame !== parent.page.mainFrame());
    await child.waitForSelector('#plain');
    await install(parent.context, child);
    await parent.page.locator('#parent-field').click();
    await child.locator('#plain').click();
    const prepared = await parent.call('prepare', {id: ++transaction, text: 'Z', refocus: true, retry: false});
    assert.equal(prepared.state, 'notfocused');
    check(await child.evaluate(() => document.activeElement === document.querySelector('#plain')));
    check(await parent.page.evaluate(() => document.activeElement.tagName === 'IFRAME'));
  }, 'frames.html');
});

test('tests that mismatched transaction ids cannot reuse verification or retry another paste', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    const id = ++transaction;
    await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
    const wrong = await agent.call('verify', {id: id + 1000, windowMs: 50});
    evidenceShape(wrong);
    check(wrong.gone && !wrong.changed && wrong.beforeInput === 0 && wrong.input === 0);
    const retry = await agent.call('prepare', {id: id + 1000, text: 'Z', refocus: true, retry: true});
    assert.equal(retry.state, 'notfocused');
    check(await agent.page.evaluate(() => probe.pastes === 1 && document.querySelector('#plain').value === 'abcdef'));
  });
});

test('tests that stopping an agent resolves pending verification as gone', async () => {
  await fixture(async agent => {
    await agent.page.locator('#blocker').click();
    const id = ++transaction;
    await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
    const verifying = agent.call('verify', {id, windowMs: 500});
    await delay(30);
    await agent.call('stop');
    const evidence = await Promise.race([verifying, delay(300).then(() => null)]);
    check(evidence?.gone, 'stop must settle outstanding verification before its original deadline');
  });
});

test('tests that reinjection replaces old listeners and discards old transaction memory', async () => {
  await fixture(async agent => {
    await agent.page.locator('#plain').click();
    const id = ++transaction;
    await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
    await agent.evaluate(source);
    const old = await agent.call('verify', {id, windowMs: 50});
    check(old.gone && !old.changed);
    const before = agent.reports.filter(r => r.k === 'tap').length;
    await agent.page.locator('#textarea').click();
    assert.equal(agent.reports.filter(r => r.k === 'tap').length - before, 1);
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'ready');
    check(result.evidence.changed);
    check(await agent.page.evaluate(() => probe.pastes === 2));
  });
});

test('tests that a handled OTP node replacement counts as landing without input events', async () => {
  await fixture(async agent => {
    await agent.page.locator('#otp0').evaluate(el => el.addEventListener('paste', event => {
      event.stopImmediatePropagation(); event.preventDefault();
      const old = el.nextElementSibling, replacement = old.cloneNode();
      replacement.value = 'Z';
      old.replaceWith(replacement);
    }, true));
    await agent.page.locator('#otp0').click();
    const result = await paste(agent);
    assert.equal(result.prepared.state, 'handled');
    check(result.evidence.changed, 'replacing an observed sibling with a populated field must count as landing');
    assert.equal(result.evidence.input, 0);
    check(await agent.page.evaluate(() => document.querySelector('#otp0').value === '' && document.querySelector('#otp0').nextElementSibling.value === 'Z'));
  });
});

test('tests that CSS-hidden inputs cannot turn their enclosing page into a drawn field', async () => {
  await fixture(async agent => {
    await eventually(() => lastBoxes(agent), 'hidden-field fixture must report geometry');
    assert.deepEqual(lastBoxes(agent).boxes, []);
    assert.equal(lastBoxes(agent).focused, null);
  }, 'hidden-inputs.html');
});

test('tests that an unfocused cross-site child reports fields before its first tap', async () => {
  await fixture(async parent => {
    const child = parent.page.frames().find(frame => frame !== parent.page.mainFrame());
    await child.waitForSelector('#plain');
    await parent.page.locator('#parent-field').focus();
    check(await child.evaluate(() => !document.hasFocus()), 'child must start unfocused');
    const agent = await install(parent.context, child);
    await eventually(() => lastBoxes(agent)?.boxes.length > 0, 'unfocused child fields must be available before the first tap');
    assert.equal(lastBoxes(agent).focused, null);
    await child.locator('#plain').click();
    await eventually(() => lastBoxes(agent)?.focused !== null, 'child focus must be reported');
    await parent.page.locator('#parent-field').focus();
    await eventually(() => lastBoxes(agent)?.focused === null, 'blurred child must clear its focus index');
    check(lastBoxes(agent).boxes.length > 0, 'blurred child must retain visible field geometry');
  }, 'frames.html');
});

test('tests that a CSS-hidden iframe cannot contribute field geometry', async () => {
  await fixture(async parent => {
    await parent.page.locator('iframe').evaluate(el => { el.style.visibility = 'hidden'; });
    assert.deepEqual(await parent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [0, 0, 0, 0]);
    await parent.page.locator('iframe').evaluate(el => { el.style.visibility = ''; });
    assert.deepEqual(await parent.evaluate('__sv.frameRect(document.querySelector("iframe"))'), [47, 67, 450, 650]);
  }, 'frames.html');
});

for (const method of ['prepare', 'verify']) {
  test(`tests that ${method} flushes pending input metadata before its CDP reply`, async () => {
    await fixture(async agent => {
      await agent.page.locator('#plain').click();
      const id = ++transaction;
      if (method === 'verify') await agent.call('prepare', {id, text: 'Z', refocus: false, retry: false});
      const arg = method === 'prepare' ? {id, text: 'Z', refocus: false, retry: false} : {id, windowMs: 0};
      await agent.evaluate(`(() => {
        const field = document.querySelector('#plain');
        field.dispatchEvent(new InputEvent('input', {bubbles: true}));
        field.dispatchEvent(new InputEvent('input', {bubbles: true}));
        return __sv.${method}(${JSON.stringify(arg)});
      })()`);
      assert.equal(agent.reports.filter(report => report.k === 'input').length, 2,
        'the transaction boundary must deliver the queued report before returning');
      await delay(70);
      assert.equal(agent.reports.filter(report => report.k === 'input').length, 2,
        'flushing must cancel the old trailing timer');
    });
  });
}
