// page.emulateMedia through Neo and through Playwright on one BrowserOS neo, each case on a
// fresh tab of tests/pages/emulation.html: what the script returns, and the media queries and
// CSS colour the page itself reports once the call resolved.
//
// Playwright cases (pin 26137face): `tests/page/page-emulate-media.spec.ts` "EM-001 should
// emulate type" (print, `{}` keeps it, `null` resets it), "EM-003 should emulate colorScheme
// should work", "EM-007 should change the actual colors in css", "EM-008 should emulate reduced
// motion", "EM-010 should emulate forcedColors", "EM-011 should emulate contrast", and
// "EM-002 should throw in case of bad media argument" (rejected, nothing changed). A later call
// keeps what an earlier one set ("EM-009 should keep reduced motion and color emulation"). An
// out-of-process iframe follows its page, whether the script reached it before the call or after.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/emulation.html': fixturePage('emulation.html'),
  // The emulation page in an iframe of the other site (127.0.0.1 and localhost are different
  // sites, so BrowserOS runs the frame out of process).
  '/outer.html': `<title>outer</title><script>
const url = new URL(location.href)
url.hostname = url.hostname === '127.0.0.1' ? 'localhost' : '127.0.0.1'
url.pathname = '/emulation.html'
document.write('<iframe src="' + url.href + '"></iframe>')
</script>`,
}

/** Reads the out-of-process frame's reporter until `done`, from the outer page. */
const FRAME_READ = `const frameState = async (done) => {
  const until = Date.now() + 3000
  for (;;) {
    const s = JSON.parse(await page.frameLocator('iframe').locator('#state').textContent())
    if (done(s) || Date.now() > until) return s
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
}
`

const cases = [
  {
    name: 'media type: print, kept by {}, reset by null',
    source: `const read = (s) => ({ print: s.media.print, screen: s.media.screen })
const before = read(await state())
await page.emulateMedia({ media: 'print' })
const print = read(await settle((s) => s.media.print))
await page.emulateMedia({})
const kept = read(await state())
await page.emulateMedia({ media: null })
const reset = read(await settle((s) => !s.media.print))
return { before, print, kept, reset }`,
    expected: {
      before: { print: false, screen: true },
      print: { print: true, screen: false },
      kept: { print: true, screen: false },
      reset: { print: false, screen: true },
    },
  },
  {
    name: 'every feature, and a later call keeps the earlier ones',
    source: `const features = ({ media }) => ({ print: media.print, dark: media['(prefers-color-scheme: dark)'],
  light: media['(prefers-color-scheme: light)'], reduce: media['(prefers-reduced-motion: reduce)'],
  forced: media['(forced-colors: active)'], more: media['(prefers-contrast: more)'] })
await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce', forcedColors: 'active', contrast: 'more' })
const set = await settle((s) => s.media['(prefers-color-scheme: dark)'] && s.media['(prefers-contrast: more)'])
await page.emulateMedia({ media: 'print' })
const later = await settle((s) => s.media.print)
return { set: features(set), color: set.color, later: features(later) }`,
    expected: {
      set: {
        print: false,
        dark: true,
        light: false,
        reduce: true,
        forced: true,
        more: true,
      },
      color: 'rgb(255, 255, 255)',
      later: {
        print: true,
        dark: true,
        light: false,
        reduce: true,
        forced: true,
        more: true,
      },
    },
  },
  {
    name: 'an out-of-process frame the script reached first',
    path: '/outer.html',
    source: `${FRAME_READ}const before = (await frameState(() => true)).media.print
await page.emulateMedia({ media: 'print', colorScheme: 'dark' })
const after = await frameState((s) => s.media.print)
return { before, print: after.media.print, dark: after.media['(prefers-color-scheme: dark)'] }`,
    expected: { before: false, print: true, dark: true },
  },
  {
    name: 'an out-of-process frame reached after the call',
    path: '/outer.html',
    source: `${FRAME_READ}await page.emulateMedia({ media: 'print' })
const after = await frameState((s) => s.media.print)
return after.media.print`,
    expected: true,
  },
  {
    name: 'a bad value is refused and nothing changes',
    source: `const before = await state()
const errors = [
  await page.emulateMedia({ media: 'bad' }).then(() => null, errorCategory),
  await page.emulateMedia({ colorScheme: 'bad' }).then(() => null, errorCategory),
]
const after = await state()
return { errors, unchanged: JSON.stringify(before.media) === JSON.stringify(after.media) }`,
    expected: { errors: ['Error', 'Error'], unchanged: true },
  },
]

test('emulateMedia: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, path = '/emulation.html', source, expected } of cases) {
      const reference = outcomeValue(await playwright(path, source))
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo(path, source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})
