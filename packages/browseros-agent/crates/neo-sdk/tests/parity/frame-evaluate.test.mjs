// frame.evaluate and frame.evaluateHandle through frames (frames.mjs, frame-top.html), Playwright's
// tab first, then Neo's: each frame's evaluation runs in that frame's own document and main world,
// a same-process child and a cross-site out-of-process frame (OOPIF) included, and changes only
// that frame; a JS handle of one frame is refused in another. Values and frames' facts compared
// as frames.mjs's `settle` reports them; errors by category.
//
// Cases from tests/page/frame-evaluate.spec.ts at 26137face: FR-001 "should have different
// execution contexts", FR-002 "should have correct execution contexts", FR-006 "should not allow
// cross-frame js handles", FR-013 "evaluateHandle should work"; and the OOPIF counterpart of
// FR-001 on frame-top.html's cross-site frame.
//
// Not covered, recorded in the slice report as `partial`: FR-007 "should allow cross-frame element
// handles" (Playwright adopts a same-process child's element into the parent's main world, which
// needs that world's execution context id; Neo, without Runtime.enable, refuses the handle as a
// foreign context's).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  failed,
  value,
} from './frames.mjs'
import { skip } from './harness.mjs'

const byName = (name) =>
  `page.frames().find((frame) => frame.name() === ${JSON.stringify(name)})`

const cases = [
  {
    name: 'each frame evaluates in its own document',
    source: `
      const same = ${byName('same')}
      const cross = ${byName('cross')}
      const inner = ${byName('chain-inner')}
      return {
        top: await settle(() => page.mainFrame().evaluate(() => [window.__who, location.pathname])),
        same: await settle(() => same.evaluate(() => [window.__who, location.search])),
        cross: await settle(() => cross.evaluate(() => [window.__who, location.hostname])),
        inner: await settle(() => inner.evaluate(() => window.__who)),
        sameText: await settle(() => same.evaluate(() => document.querySelector('#who').textContent)),
      }`,
    expected: {
      top: value(['top', '/frame-top.html']),
      same: value(['same', '?who=same']),
      cross: value(['cross', 'localhost']),
      inner: value('chain-inner'),
      sameText: value('same'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    name: 'a global set in one frame stays in that frame',
    source: `
      const same = ${byName('same')}
      const cross = ${byName('cross')}
      await page.mainFrame().evaluate(() => { window.FOO = 'top-foo' })
      await same.evaluate(() => { window.FOO = 'same-foo' })
      await cross.evaluate(() => { window.FOO = 'cross-foo' })
      return {
        top: await settle(() => page.evaluate(() => window.FOO)),
        same: await settle(() => same.evaluate(() => window.FOO)),
        cross: await settle(() => cross.evaluate(() => window.FOO)),
        chain: await settle(() => ${byName('chain')}.evaluate(() => typeof window.FOO)),
      }`,
    expected: {
      top: value('top-foo'),
      same: value('same-foo'),
      cross: value('cross-foo'),
      chain: value('undefined'),
    },
    probe: '({ ...window.__facts(), foo: window.FOO ?? null })',
    effect: (effect) => {
      assert.equal(effect.top.foo, 'top-foo')
      assert.equal(effect.same.foo, 'same-foo')
      assert.equal(effect.cross.foo, 'cross-foo')
      for (const who of [
        'grandchild',
        'chain',
        'chain-inner',
        'hidden-same',
        'hidden-cross',
      ])
        assert.equal(effect[who].foo, null, who)
    },
  },
  {
    name: 'a frame writes only its own document',
    source: `
      const cross = ${byName('cross')}
      await cross.evaluate(() => { document.querySelector('#search').value = 'typed by evaluate' })
      return await settle(() => ${byName('same')}.evaluate(() => document.querySelector('#search').value))`,
    expected: value(''),
    effect: (effect) => {
      const { cross } = assertDecoysUntouched(effect, ['cross'])
      assert.equal(cross.search, 'typed by evaluate')
    },
  },
  {
    name: 'a JS handle of one frame is refused in another',
    source: `
      const same = ${byName('same')}
      const handle = await page.evaluateHandle(() => {
        const foo = { bar: 'baz' }
        document.querySelector('#same').contentWindow.__foo = foo
        return foo
      })
      return {
        childSees: await settle(() => same.evaluate(() => window.__foo)),
        refused: await settle(() => same.evaluate((foo) => foo.bar, handle)),
        parentAccepts: await settle(() => page.evaluate((foo) => foo.bar, handle)),
      }`,
    expected: {
      childSees: value({ bar: 'baz' }),
      refused: failed('error'),
      parentAccepts: value('baz'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    name: 'frame.evaluateHandle returns a handle in that frame',
    source: `
      const cross = ${byName('cross')}
      const windowHandle = await cross.evaluateHandle(() => window)
      return {
        preview: windowHandle.toString(),
        who: await settle(() => windowHandle.evaluate((w) => w.__who)),
        json: await settle(async () => (await cross.evaluateHandle(() => ({ who: window.__who }))).jsonValue()),
      }`,
    expected: {
      preview: 'Window',
      who: value('cross'),
      json: value({ who: 'cross' }),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('frame.evaluate lands in the frame it names, OOPIFs included', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
