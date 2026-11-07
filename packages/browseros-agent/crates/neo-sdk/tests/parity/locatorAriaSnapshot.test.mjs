// Neo: compare scoped snapshots and ref actions with Playwright on fresh fixture pages.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('tests that locator snapshots match Playwright and scoped refs work', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/locator-snapshot.html', import.meta.url),
      'utf8',
    ),
    source: `
      const section = page.getByRole('main').locator('section').filter({ hasText: 'Featured' })
      const plain = await section.ariaSnapshot()
      const shallow = await section.ariaSnapshot({ depth: 2 })
      const ai = await section.ariaSnapshot({ mode: 'ai' })
      const ref = ai.split('\\n').find(line => line.includes('button "Buy apple"'))
        ?.match(/\\[ref=(e\\d+)\\]/)?.[1]
      await page.locator('aria-ref=' + ref).click()
      const refText = await page.locator('aria-ref=' + ref).textContent()
      const strict = []
      for (const mode of ['default', 'ai'])
        strict.push(await page.getByRole('button').ariaSnapshot({ mode }).catch(errorCategory))
      const noMatch = await page.getByRole('region', { name: 'Missing' })
        .ariaSnapshot({ mode: 'ai' }).catch(errorCategory)
      return { plain, shallow, ai, refText, strict, noMatch }
    `,
    probe: () => document.querySelector('#result').textContent,
  })
  assert.doesNotMatch(outcome.value.plain, /Outside|Other products/)
  assert.notEqual(outcome.value.shallow, outcome.value.plain)
  assert.match(outcome.value.ai, /\[ref=e\d+\]/)
  assert.equal(outcome.value.refText, 'Buy apple')
  assert.deepEqual(outcome.value.strict, ['Error:strict', 'Error:strict'])
  assert.equal(outcome.value.noMatch, 'Error')
})

/**
 * `ai` locator snapshots over frames, on tabs of frame-top.html (frames.mjs). Their refs are
 * page-wide, as Playwright's: each is used through `page.locator('aria-ref=…')`, from the main
 * frame, and must act on the element the snapshot showed, in its own frame, never on the
 * same-looking widget of another frame (every frame repeats the same ids, so a wrong frame shows
 * up as a changed decoy). A subtree holding an iframe has that frame's document stitched in, out
 * of process and nested included. The YAML is compared with refs blanked; ref strings differ by
 * design (Playwright: `f<frame number>e<n>`; Neo: the iframe refs on the way, `f<n>…e<n>`).
 */
const frameCases = [
  {
    name: 'a frame locator snapshot names its elements from the main frame',
    source: `const refOf = (yaml, text) => /\\[ref=([^\\]]+)\\]/.exec(yaml.split('\\n').find((line) => line.includes(text)) ?? '')?.[1]
    const blank = (yaml) => yaml.replace(/\\[ref=[^\\]]+\\]/g, '[ref]')
    // Each snapshot replaces its frame's refs, in Playwright too, so each ref is used before the
    // next snapshot of the same frame.
    const viaFrameLocator = await page.frameLocator('#login-iframe').locator('#send').ariaSnapshot({ mode: 'ai' })
    const send = refOf(viaFrameLocator, 'button "Send"')
    const text = await settle(() => page.locator('aria-ref=' + send).textContent())
    const clicked = await settle(() => page.locator('aria-ref=' + send).click())
    const viaFrame = await page.frame('cross').locator('#email').ariaSnapshot({ mode: 'ai' })
    const email = refOf(viaFrame, 'textbox')
    const filled = await settle(() => page.locator('aria-ref=' + email).fill('via-frame-ref'))
    return {
      yaml: [blank(viaFrameLocator), blank(viaFrame)],
      text,
      clicked,
      filled,
      refs: { send, email },
    }`,
    perEngine: ['refs'],
    expected: {
      yaml: [
        '- button "Send" [ref]',
        '- textbox "Email" [ref]: cross@seed.test',
      ],
      text: value('Send'),
      clicked: 'undefined',
      filled: 'undefined',
    },
    effect: (effect) => {
      const { cross } = assertDecoysUntouched(effect, ['cross'])
      assert.equal(cross.email, 'via-frame-ref')
      assert.deepEqual(
        cross.events.filter((event) => event.type === 'click'),
        [{ type: 'click', target: 'send', trusted: true }],
        'the click reached the cross frame’s Send',
      )
    },
    checkNeo: ({ result }) => {
      for (const ref of Object.values(result.value.refs))
        assert.match(ref, /^f\d+e\d+$/, 'a path through the cross-site iframe')
    },
  },
  {
    name: 'an ai locator snapshot stitches the frames inside its subtree',
    source: `const refOf = (yaml, marker, text) => {
      const lines = yaml.split('\\n')
      return /\\[ref=([^\\]]+)\\]/.exec(lines.slice(lines.findIndex((line) => line.includes(marker))).find((line) => line.includes(text)) ?? '')?.[1]
    }
    const blank = (yaml) => yaml.replace(/\\[ref=[^\\]]+\\]/g, '[ref]')
    const wrap = await page.locator('#cross-wrap').ariaSnapshot({ mode: 'ai' })
    const send = refOf(wrap, ': grandchild gen 1', 'button "Send"')
    return {
      wrap: blank(wrap),
      clicked: send === undefined ? 'no ref' : await settle(() => page.locator('aria-ref=' + send).click()),
      refs: { send },
    }`,
    perEngine: ['refs'],
    expected: {
      wrap: (wrap) => {
        for (const who of ['cross', 'grandchild'])
          assert.ok(
            wrap.includes(`- paragraph [ref]: ${who} gen 1`),
            `${who}'s document is stitched in`,
          )
        assert.doesNotMatch(
          wrap,
          /(same|chain|top) gen 1/,
          'only the frames inside the subtree',
        )
      },
      clicked: 'undefined',
    },
    effect: (effect) => {
      const { grandchild } = assertDecoysUntouched(effect, ['grandchild'])
      assert.deepEqual(
        grandchild.events.filter((event) => event.type === 'click'),
        [{ type: 'click', target: 'send', trusted: true }],
        'the click reached the grandchild’s Send',
      )
    },
    checkNeo: ({ result }) =>
      assert.match(
        result.value.refs.send,
        /^f\d+f\d+e\d+$/,
        'two iframes on the way',
      ),
  },
]

test('ai locator snapshots over frames answer page-wide refs through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})

const REVIEW_REFS_PAGE = `<button id="one">One</button><button id="two">Two</button>
  <iframe id="child" name="child" srcdoc="<button>Inner</button>"></iframe>`
const REVIEW_REF_HELPER = `const refOf = (snapshot, label) => {
  const line = snapshot.split(String.fromCharCode(10)).find(line => line.includes('button "' + label + '"'))
  if (!line || !line.includes('[ref=')) throw new Error('missing ref for ' + label)
  return line.split('[ref=')[1].split(']')[0]
};`

test('review: child refs survive an unrelated parent locator snapshot', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: REVIEW_REFS_PAGE,
    source:
      REVIEW_REF_HELPER +
      `
      const full = await page.ariaSnapshot({ mode: 'ai' })
      const inner = refOf(full, 'Inner')
      await page.locator('#one').ariaSnapshot({ mode: 'ai' })
      const old = page.locator('aria-ref=' + inner)
      const count = await old.count()
      const text = await old.textContent({ timeout: 500 }).catch(errorCategory)
      return { count, text }
    `,
    probe: 'document.querySelector("#two").textContent',
  })
})

test('review: child snapshots do not revive stale parent refs', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: REVIEW_REFS_PAGE,
    source:
      REVIEW_REF_HELPER +
      `
      const full = await page.ariaSnapshot({ mode: 'ai' })
      const two = page.locator('aria-ref=' + refOf(full, 'Two'))
      await page.locator('#one').ariaSnapshot({ mode: 'ai' })
      const before = await two.count()
      const child = await page.frameLocator('#child').locator('button').ariaSnapshot({ mode: 'ai' })
      const inner = await page.locator('aria-ref=' + refOf(child, 'Inner')).textContent()
      return { before, after: await two.count(), inner }
    `,
    probe: 'document.querySelector("#two").textContent',
  })
})

const refHelper = `const refOf = snapshot => snapshot.match(/\\[ref=([^\\]]+)\\]/)?.[1];`

// A `visibility: hidden` iframe gets no ref, so Neo names its frame by identity (`fi…`), as
// Playwright names it by number; both answer refs that route.
test('review: ai snapshots through a direct Frame behind a hidden iframe answer routable refs', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<iframe name="hidden" style="visibility:hidden" srcdoc="<button>Hidden</button>"></iframe>',
    source:
      refHelper +
      `
      const target = page.frame('hidden').locator('button')
      const plain = await target.ariaSnapshot()
      const ai = await target.ariaSnapshot({ mode: 'ai' })
      const routed = page.locator('aria-ref=' + refOf(ai))
      return {
        plainHasButton: plain.includes('Hidden'),
        ai: { hasButton: ai.includes('Hidden'), hasRef: !!refOf(ai) },
        routed: { count: await routed.count(), text: await routed.textContent() },
      }
    `,
    probe: 'document.querySelector("iframe").style.visibility',
  })
  assert.deepEqual(outcome.value.routed, { count: 1, text: 'Hidden' })
})

test('a fresh hidden owner keeps parent refs and its frame answers refs that route', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button>Outer</button><output id="effect">untouched</output>
      <iframe name="hidden" style="visibility:hidden" srcdoc="<button onclick=&quot;parent.document.getElementById('effect').textContent='changed'&quot;>Inner</button>"></iframe>`,
    source:
      refHelper +
      `
      const top = await page.locator('body > button').ariaSnapshot({mode:'ai'});
      const before = await page.locator('aria-ref=' + refOf(top)).count();
      const frame = page.frame('hidden');
      await frame.locator('button').waitFor({state:'attached'});
      const snapshot = await frame.locator('button').ariaSnapshot({mode:'ai'});
      const inner = page.locator('aria-ref=' + refOf(snapshot));
      return {
        before,
        after: await page.locator('aria-ref=' + refOf(top)).count(),
        text: snapshot.includes('Inner'),
        routed: await inner.textContent(),
        // The hidden iframe takes no pointer: a click through the ref never lands.
        click: await inner.click({ timeout: 1000 }).then(() => 'clicked', errorCategory),
      };
    `,
    probe: 'document.querySelector("#effect").textContent',
  })
  assert.deepEqual(outcome.value, {
    before: 1,
    after: 1,
    text: true,
    routed: 'Inner',
    click: 'TimeoutError',
  })
})

// The frame of a hidden cross-site (out-of-process) iframe: its refs route; after the frame
// navigates, the old document's refs match nothing and a new snapshot's refs name the same frame;
// once the iframe is gone, a ref naming its frame fails at once.
test('a hidden out-of-process frame answers refs through navigation and fails them once gone', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<iframe name="remote" style="visibility:hidden"></iframe><script>
      if (location.search.startsWith('?inner')) document.body.innerHTML = '<button>Remote</button>'
      else document.querySelector('iframe').src = 'http://localhost:' + location.port + '/method.html?inner'
    </script>`,
    source:
      refHelper +
      `
      const button = page.frameLocator('iframe[name=remote]').locator('button')
      await button.waitFor({ state: 'attached' })
      const first = refOf(await page.frame('remote').locator('button').ariaSnapshot({ mode: 'ai' }))
      const routed = await page.locator('aria-ref=' + first).textContent()
      await page.evaluate(() => {
        document.querySelector('iframe').src = 'http://localhost:' + location.port + '/method.html?inner-again'
      })
      await page.frame('remote').waitForURL(/inner-again/)
      await button.waitFor({ state: 'attached' })
      const afterNavigation = await page.locator('aria-ref=' + first).count()
      const second = refOf(await page.frame('remote').locator('button').ariaSnapshot({ mode: 'ai' }))
      const frameOf = (ref) => ref.replace(/e\\d+$/, '')
      const result = {
        routed,
        afterNavigation,
        sameFrame: frameOf(first) === frameOf(second),
        again: await page.locator('aria-ref=' + second).textContent(),
      }
      await page.evaluate(() => document.querySelector('iframe').remove())
      // Once the page knows the frame is gone (before, Playwright's jump still finds it, and the
      // query in the dying frame counts 0), the ref is an invalid selector.
      while (page.frame('remote')) await new Promise((resolve) => setTimeout(resolve, 20))
      result.gone = await page.locator('aria-ref=' + second).count().then(String, errorCategory)
      return result
    `,
    probe: 'document.querySelectorAll("iframe").length',
  })
  assert.deepEqual(outcome.value, {
    routed: 'Remote',
    afterNavigation: 0,
    sameFrame: true,
    again: 'Remote',
    gone: 'Error',
  })
})

test('named owners can become hidden and pointer-events-none owners remain routable', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: `<button id="hide" onclick="document.querySelector('#warm').style.visibility='hidden'">Hide</button>
      <iframe id="warm" name="warm" srcdoc="<button>Warm</button>"></iframe>
      <iframe name="passive" style="pointer-events:none" srcdoc="<button>Passive</button>"></iframe>`,
    source:
      refHelper +
      `
      const warm = page.frame('warm').locator('button');
      await warm.waitFor();
      await warm.ariaSnapshot({mode:'ai'});
      await page.locator('#hide').click();
      const hidden = await warm.ariaSnapshot({mode:'ai'});
      const passive = await page.frame('passive').locator('button').ariaSnapshot({mode:'ai'});
      return {
        hidden:await page.locator('aria-ref=' + refOf(hidden)).textContent(),
        passive:await page.locator('aria-ref=' + refOf(passive)).textContent()
      };
    `,
    probe: 'document.querySelector("#warm").style.visibility',
  })
})

test('a direct Frame in a closed shadow root returns a routable AI ref', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: `<div id="host"></div><script>
      const shadow = document.querySelector('#host').attachShadow({mode:'closed'});
      const iframe = document.createElement('iframe');
      iframe.name = 'closed'; iframe.srcdoc = '<button>Closed child</button>';
      shadow.appendChild(iframe);
    </script>`,
    source:
      refHelper +
      `
      const frame = page.frame('closed');
      if (!frame) throw new Error('missing closed-shadow frame');
      const button = frame.locator('button');
      await button.waitFor();
      const snapshot = await button.ariaSnapshot({mode:'ai'});
      return {
        direct:await button.textContent(),
        frameScoped:/^f/.test(refOf(snapshot) ?? ''),
        ownerMatches:await page.locator('iframe').count(),
        routed:await page.locator('aria-ref=' + refOf(snapshot)).count(),
      };
    `,
    probe: 'document.querySelector("#host").shadowRoot === null',
  })
})

// Playwright's `performActionPreChecks` runs before a default-mode locator snapshot: a pending
// main-frame navigation is waited out and logged, then the snapshot reads the next document.
// `/slow` answers after 2 s; the old page has an `h1` of its own. The navigation is the run's
// first browser call, a `page.evaluate`, so Neo follows it from its request as Playwright does.
// Neo read the next document before too (its reads of the old one poll again), so the value
// alone cannot tell; Neo's call log shows the pre-check's wait, in Playwright's words.
test('a locator snapshot waits out a pending navigation first, as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<!doctype html><html><body><h1>old</h1></body></html>',
    slowMs: 2000,
    source: `
      await page.evaluate(() => { location.href = '/slow' })
      await new Promise((resolve) => setTimeout(resolve, 300))
      return await page.locator('h1').ariaSnapshot()
    `,
    probe: 'document.querySelector("h1").textContent',
  })
  assert.equal(outcome.value, '- heading "/slow" [level=1]')
  const [snapshot] = outcome.calls.filter((c) => c.method === 'ariaSnapshot')
  assert.ok(
    snapshot.log.some((line) =>
      /waiting for ".*\/slow" navigation to finish\.\.\./.test(line),
    ),
    JSON.stringify(snapshot.log),
  )
})
