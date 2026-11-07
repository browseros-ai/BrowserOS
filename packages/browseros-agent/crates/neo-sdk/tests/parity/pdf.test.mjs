// page.pdf through Neo and through real Playwright on one BrowserOS neo, each case on two fresh
// tabs serving the same page (capture.mjs). Compared: the PDF facts the script read from its own
// bytes (`%PDF-` header, page count, first media box in points, outline and tag trees), the
// error's kind, and the `path` file. Never bytes across engines (a PDF carries its creation
// time) or error wording.
//
// Cases from Playwright's tests/library/pdf.spec.ts at 26137face, named by their ids, plus the
// option mapping of server/chromium/crPdf.ts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileChecksum, withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const headings = `<!doctype html><title>headings</title>${Array.from(
  { length: 60 },
  (_, i) =>
    `<h${(i % 3) + 1}>Heading ${i}</h${(i % 3) + 1}><p>${'Lorem ipsum dolor sit amet. '.repeat(30)}</p>`,
).join('')}`

const cases = [
  {
    name: 'PD-001 should be able to save file: letter by default',
    source: "return describePdf(await page.pdf({ path: at('output.pdf') }))",
    file: 'output.pdf',
  },
  {
    name: 'PD-002 should be able to generate outline',
    source: `return {
      plain: describePdf(await page.pdf()),
      outlined: describePdf(await page.pdf({ tagged: true, outline: true })),
    }`,
  },
  {
    name: 'format, landscape and pageRanges',
    source:
      "return describePdf(await page.pdf({ format: 'A4', landscape: true, pageRanges: '1-2' }))",
    expect: { pages: 2, mediaBox: [0, 0, 843, 596] },
  },
  {
    name: 'number sizes are CSS pixels, string sizes keep their units',
    source:
      "return describePdf(await page.pdf({ width: 480, height: '5in', margin: { top: 20, bottom: '1cm', left: '0.5in' }, pageRanges: '1' }))",
    expect: { pages: 1, mediaBox: [0, 0, 360, 360] },
  },
  {
    name: 'scale, printBackground, header and footer templates',
    source: `return describePdf(await page.pdf({
      scale: 0.5, printBackground: true, displayHeaderFooter: true,
      headerTemplate: '<span class=title></span>', footerTemplate: '<span class=pageNumber></span>',
      margin: { top: '1in', bottom: '1in' }, preferCSSPageSize: true,
    }))`,
  },
  {
    name: 'an unknown paper format is an Error, and writes nothing',
    source: `try { await page.pdf({ format: 'A11', path: at('none.pdf') }); return 'printed' } catch (error) { return errorKind(error) }`,
    noFile: 'none.pdf',
    expect: { error: { name: 'Error', strict: false } },
  },
  {
    name: 'a page range past the last page is an Error',
    source: `try { await page.pdf({ pageRanges: '9999' }); return 'printed' } catch (error) { return errorKind(error) }`,
    expect: { error: { name: 'Error', strict: false } },
  },
  {
    name: 'a unit that is not px, in, cm or mm is an Error',
    source: `try { await page.pdf({ width: '10pt' }); return 'printed' } catch (error) { return errorKind(error) }`,
    expect: { error: { name: 'Error', strict: false } },
  },
]

/** `value` without checksums: bytes are compared within an engine, never across them. */
function facts(value) {
  if (Array.isArray(value)) return value.map(facts)
  if (value && typeof value === 'object')
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) => key !== 'checksum')
        .map(([key, item]) => [key, facts(item)]),
    )
  return value
}

function seen(engine) {
  return engine.error ? { error: engine.error } : { value: facts(engine.value) }
}

test('page.pdf reads the same through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withEngines(
    t,
    { pages: { '/headings.html': headings } },
    async (run) => {
      for (const c of cases) {
        await t.test(c.name, async () => {
          const { playwright, neo } = await run('/headings.html', c.source)
          t.diagnostic(`${c.name}: ${JSON.stringify(seen(neo))}`)
          assert.deepEqual(seen(neo), seen(playwright), 'what the script saw')
          if (c.expect) {
            const value = neo.value
            for (const [key, expected] of Object.entries(c.expect))
              assert.deepEqual(
                value[key],
                expected,
                `the case is the one it names: ${key}`,
              )
          }
          if (c.file) {
            for (const [side, engine] of [
              ['Playwright', playwright],
              ['Neo', neo],
            ]) {
              const written = engine.file(c.file)
              assert.ok(written, `${side} wrote ${c.file}`)
              assert.equal(
                fileChecksum(written),
                engine.value.checksum,
                `${side}'s ${c.file} holds the returned bytes`,
              )
            }
            assert.equal(
              neo.outcome.artifacts.length,
              1,
              "Neo's outcome lists the file",
            )
          }
          if (c.noFile) {
            assert.equal(
              playwright.file(c.noFile),
              null,
              'Playwright wrote nothing',
            )
            assert.equal(neo.file(c.noFile), null, 'Neo wrote nothing')
          }
        })
      }
    },
  )
})
