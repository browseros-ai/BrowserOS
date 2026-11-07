// ElementHandle.ownerFrame and contentFrame through frames (frames.mjs, frame-top.html),
// Playwright's tab first, then Neo's. A handle's owner frame is the run's own Frame object for
// the frame whose document holds the element, also once it is detached. A frame element's
// content frame is the frame it shows, for a same-process child and an out-of-process frame
// (OOPIF); any other element has none. Frames compared by identity inside the script.
//
// Cases from tests/page/elementhandle-owner-frame.spec.ts at 26137face: EH-055 "should work",
// EH-056 "should work for cross-process iframes", EH-057 "should work for document", EH-060
// "should work for detached elements"; elementhandle-content-frame.spec.ts EH-019 "should work",
// EH-020 "should work for cross-process iframes", EH-022 "should return null for non-iframes".
import { test } from 'node:test'
import { compareFrameCases, value } from './frames.mjs'
import { skip } from './harness.mjs'

const byName = (name) =>
  `page.frames().find((frame) => frame.name() === ${JSON.stringify(name)})`

const cases = [
  {
    name: 'owner and content frames are the run’s frame objects',
    source: `
      const same = ${byName('same')}
      const cross = ${byName('cross')}
      const sameElement = await page.$('#same')
      const crossElement = await page.$('#login-iframe')
      const top = await page.$('#who')
      const inSame = await same.$('#who')
      const inCross = await cross.$('#who')
      const document = await page.evaluateHandle(() => document)
      return {
        topOwner: (await top.ownerFrame()) === page.mainFrame(),
        sameOwner: (await inSame.ownerFrame()) === same,
        crossOwner: (await inCross.ownerFrame()) === cross,
        documentOwner: (await document.asElement().ownerFrame()) === page.mainFrame(),
        sameContent: (await sameElement.contentFrame()) === same,
        crossContent: (await crossElement.contentFrame()) === cross,
        notFrame: await settle(() => top.contentFrame()),
      }`,
    expected: {
      topOwner: true,
      sameOwner: true,
      crossOwner: true,
      documentOwner: true,
      sameContent: true,
      crossContent: true,
      notFrame: value(null),
    },
  },
  {
    name: 'a detached element keeps its owner frame',
    source: `
      const gone = await page.$('#not-a-frame')
      await gone.evaluate((element) => element.remove())
      return {
        owner: (await gone.ownerFrame()) === page.mainFrame(),
        content: await settle(() => gone.contentFrame()),
      }`,
    expected: { owner: true, content: value(null) },
  },
]

test('ElementHandle.ownerFrame and contentFrame', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
