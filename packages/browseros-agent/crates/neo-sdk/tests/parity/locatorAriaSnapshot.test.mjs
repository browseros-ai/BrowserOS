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

// Deviation UNNAMED_FRAME (neo-runtime/src/reads.rs): Neo names a frame's refs by the iframe refs
// from the main frame, and a `visibility: hidden` iframe gets none, so its document's AI snapshot
// throws where Playwright, which numbers frames, answers refs. The default mode is the same.
test('review: ai snapshots through a direct Frame behind a hidden iframe', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        plainHasButton: true,
        ai: { hasButton: true, hasRef: true },
      })
      assert.deepEqual(neo, { plainHasButton: true, ai: { error: 'Error' } })
    },
    html: '<iframe name="hidden" style="visibility:hidden" srcdoc="<button>Hidden</button>"></iframe>',
    source: `
      const target = page.frame('hidden').locator('button')
      const plain = await target.ariaSnapshot()
      const ai = await target.ariaSnapshot({ mode: 'ai' }).then(
        snapshot => ({ hasButton: snapshot.includes('Hidden'), hasRef: snapshot.includes('[ref=') }),
        error => ({ error: errorCategory(error) }))
      return { plainHasButton: plain.includes('Hidden'), ai }
    `,
    probe: 'document.querySelector("iframe").style.visibility',
  })
})

const refHelper = `const refOf = snapshot => snapshot.match(/\\[ref=([^\\]]+)\\]/)?.[1];`

test('fresh hidden-owner refusal preserves parent refs and page state', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: `<button>Outer</button><output id="effect">untouched</output>
      <iframe name="hidden" style="visibility:hidden" srcdoc="<button onclick=&quot;parent.document.getElementById('effect').textContent='changed'&quot;>Inner</button>"></iframe>`,
    source:
      refHelper +
      `
      const top = await page.locator('body > button').ariaSnapshot({mode:'ai'});
      const before = await page.locator('aria-ref=' + refOf(top)).count();
      const frame = page.frame('hidden');
      await frame.locator('button').waitFor({state:'attached'});
      const result = await frame.locator('button').ariaSnapshot({mode:'ai'}).then(
        s => ({text:s.includes('Inner'),ref:!!refOf(s)}),
        error => ({error:errorCategory(error)}));
      return {before, after:await page.locator('aria-ref=' + refOf(top)).count(), result};
    `,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        before: 1,
        after: 1,
        result: { text: true, ref: true },
      })
      assert.deepEqual(neo, { before: 1, after: 1, result: { error: 'Error' } })
    },
    probe: 'document.querySelector("#effect").textContent',
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
