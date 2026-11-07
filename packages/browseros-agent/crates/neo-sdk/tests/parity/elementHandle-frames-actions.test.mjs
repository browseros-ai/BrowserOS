// ElementHandle actions on elements of child frames, through the Playwright reference and through
// Neo, on tests/pages/frame-top.html: a same-process iframe (`same`) and an out-of-process one
// (`cross`, the `localhost` login iframe). Each handle comes from its frame's own `$`, so the
// action finds its element's frames from the element itself, as Playwright's
// `_checkFrameIsHitTarget` walks the element's frame and its frame elements up to the main
// frame. Every action resolves, and every frame of the tab holds what Playwright's does (the
// clicks, the check, the fill, the selection and the key landed in the right frame).
//
// From Playwright's tests/page/elementhandle-click.spec.ts at 26137face, EH-010 "should work",
// and tests/page/elementhandle-misc.spec.ts, EH-045 "should hover", EH-047 "should fill input",
// EH-049 "should check the box", EH-052 "should select single option", here in child frames.
import { test } from 'node:test'
import { compareFrameCases } from './frames.mjs'
import { skip } from './harness.mjs'

const byName = (name) =>
  `page.frames().find((frame) => frame.name() === ${JSON.stringify(name)})`

const cases = [
  {
    name: 'actions on handles in a same-process and an out-of-process frame',
    source: `
      const same = ${byName('same')}
      const cross = ${byName('cross')}
      return {
        sameClick: await settle(async () => (await same.$('#send')).click()),
        crossClick: await settle(async () => (await cross.$('#send')).click()),
        crossDblclick: await settle(async () => (await cross.$('#send')).dblclick()),
        crossCheck: await settle(async () => (await cross.$('#agree')).check()),
        sameFill: await settle(async () => (await same.$('#email')).fill('same@frame.test')),
        crossFill: await settle(async () => (await cross.$('#email')).fill('cross@frame.test')),
        sameSelect: await settle(async () => (await same.$('#size')).selectOption('l')),
        crossHover: await settle(async () => (await cross.$('#search')).hover()),
        crossPress: await settle(async () => (await cross.$('#search')).press('x')),
        crossFocus: await settle(async () => (await cross.$('#email')).focus()),
      }`,
    expected: {
      sameClick: 'undefined',
      crossClick: 'undefined',
      crossDblclick: 'undefined',
      crossCheck: 'undefined',
      sameFill: 'undefined',
      crossFill: 'undefined',
      sameSelect: { value: ['l'] },
      crossHover: 'undefined',
      crossPress: 'undefined',
      crossFocus: 'undefined',
    },
  },
]

test('ElementHandle actions in child frames match Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
