// What the screenshot, locator-screenshot and pdf parity tests share: one browser (optionally
// launched with extra switches), a fixture server, real Playwright over connectOverCDP, and Neo
// through neo_run with an artifact directory. Each case runs on two fresh tabs serving the same
// page, Playwright's first. The script reports decoded facts of its own bytes (image type, size,
// whether the PNG has an alpha channel; a PDF's header, page count and media box; a checksum),
// computed in the script by `DECODERS`, so the same code runs in Node and in Neo's QuickJS and
// no pixel or byte golden is ever compared. A `path` option names `at('<file>')`: an absolute
// path in a temporary directory for Playwright, a path relative to the run's artifact directory
// for Neo; the test then checks each engine's file against the checksum it returned.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  launchReference,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  serveFixture,
  startNeo,
  waitForUrl,
} from './harness.mjs'

/**
 * Decoders the scripts call on the bytes each engine returned. They read a `Uint8Array` by
 * index only, so Node's `Buffer` and Neo's behave the same.
 */
export const DECODERS = String.raw`
const u16be = (b, at) => (b[at] << 8) | b[at + 1]
const u32be = (b, at) => ((b[at] << 24) >>> 0) + ((b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3])
const u16le = (b, at) => b[at] | (b[at + 1] << 8)
const u24le = (b, at) => b[at] | (b[at + 1] << 8) | (b[at + 2] << 16)
function checksum(bytes) {
  let hash = 0x811c9dc5
  for (let i = 0; i < bytes.length; i++) hash = Math.imul(hash ^ bytes[i], 0x01000193) >>> 0
  return hash.toString(16) + ':' + bytes.length
}
function describeImage(bytes) {
  const sum = checksum(bytes)
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47)
    return { type: 'png', width: u32be(bytes, 16), height: u32be(bytes, 20), alpha: bytes[25] === 6 || bytes[25] === 4, checksum: sum }
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    let at = 2
    while (at < bytes.length && bytes[at] === 0xff) {
      const marker = bytes[at + 1]
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc)
        return { type: 'jpeg', width: u16be(bytes, at + 7), height: u16be(bytes, at + 5), alpha: false, checksum: sum }
      at += 2 + u16be(bytes, at + 2)
    }
    return { type: 'jpeg', checksum: sum }
  }
  const tag = (at) => String.fromCharCode(bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3])
  if (tag(0) === 'RIFF' && tag(8) === 'WEBP') {
    const chunk = tag(12)
    if (chunk === 'VP8X') return { type: 'webp', width: 1 + u24le(bytes, 24), height: 1 + u24le(bytes, 27), checksum: sum }
    if (chunk === 'VP8 ') return { type: 'webp', width: u16le(bytes, 26) & 0x3fff, height: u16le(bytes, 28) & 0x3fff, checksum: sum }
    return { type: 'webp', chunk, checksum: sum }
  }
  return { type: 'unknown', checksum: sum }
}
function describePdf(bytes) {
  let text = ''
  for (let i = 0; i < bytes.length; i += 8192) text += String.fromCharCode(...bytes.subarray(i, i + 8192))
  const pages = (text.match(/\/Type\s*\/Page(?!s)/g) || []).length
  const box = /\/MediaBox\s*\[\s*([\d.\s-]+)\]/.exec(text)
  return {
    header: text.slice(0, 5),
    pages,
    mediaBox: box ? box[1].trim().split(/\s+/).map((n) => Math.round(Number(n))) : null,
    outline: /\/Outlines/.test(text),
    tagged: /\/StructTreeRoot/.test(text),
    checksum: checksum(bytes),
  }
}
function errorKind(error) {
  return { error: { name: error.name, strict: /strict mode violation/.test(error.message) } }
}
`

/** The checksum `DECODERS` computes, over a file the test reads. */
export function fileChecksum(path) {
  const bytes = readFileSync(path)
  let hash = 0x811c9dc5
  for (const byte of bytes) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  return `${hash.toString(16)}:${bytes.length}`
}

/**
 * One browser and one fixture server serving `pages`, Playwright connected over CDP, and
 * neo_run built. `body` gets `run(path, source, probe)`, which runs `source` on two fresh tabs of
 * `path`, Playwright's first, and returns `{ playwright, neo }`: each with the script's `value`
 * or its error's kind, the page's `probe` value after the script, `file(name)` (the path the
 * engine wrote `at(name)` to, or `null` when it wrote none) and, for Neo, the run's `outcome`.
 */
export async function withEngines(t, { pages, browserArgs = [] }, body) {
  t.diagnostic(
    `Playwright ${playwrightVersion}; ${process.env.NEO_PARITY_PLAYWRIGHT ?? 'npm playwright-core'}`,
  )
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages })
    // `isLocal`: the browser shares this machine, as Neo's always does, so Playwright sets an
    // upload's paths as paths (`DOM.setFileInputFiles`) rather than reading them into buffers
    // for a remote browser (`fileUploadUtils.ts`); nothing else reads it.
    const { playwright, cdp, ...browser } = await launchReference(
      cleanups,
      browserBinary,
      { args: browserArgs, cdp: true, connect: { isLocal: true } },
    )
    const [context] = playwright.contexts()
    const scratch = mkdtempSync(join(tmpdir(), 'neo-capture-parity-'))
    cleanups.push(() => rmSync(scratch, { recursive: true, force: true }))
    let runs = 0
    await body(async (path, source, probe, files = {}) => {
      const dir = join(scratch, String(++runs))
      const url = `${origin}${path}`
      // Files the script reads: the same ones in each engine's directory (`at(name)`).
      for (const root of [join(dir, 'playwright'), join(dir, 'neo')]) {
        for (const [name, content] of Object.entries(files)) {
          const file = join(root, name)
          mkdirSync(dirname(file), { recursive: true })
          writeFileSync(file, content)
        }
      }
      return {
        playwright: await referenceHalf(() =>
          viaPlaywright(context, url, source, probe, join(dir, 'playwright')),
        ),
        neo: await viaNeo(cdp, browser.port, neoRun, url, source, probe, dir),
      }
    })
  } finally {
    await runCleanups(cleanups)
  }
}

async function viaPlaywright(context, url, source, probe, dir) {
  const page = await context.newPage()
  try {
    await page.goto(url)
    const script = `${DECODERS}\nconst at = (name) => ${JSON.stringify(`${dir}/`)} + name;\n${source}`
    const AsyncFunction = (async () => {}).constructor
    const result = await new AsyncFunction('page', 'context', script)(
      page,
      context,
    ).then(
      (value) => ({ value }),
      (error) => ({
        error: {
          name: error.name,
          strict: /strict mode violation/.test(error.message),
        },
      }),
    )
    const after = probe ? await page.evaluate(probe) : undefined
    return {
      ...result,
      probe: after,
      file: (name) => existing(join(dir, name)),
    }
  } finally {
    await page.close()
  }
}

async function viaNeo(cdp, port, neoRun, url, source, probe, dir) {
  const { targetId } = await cdp.send('Target.createTarget', { url })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  try {
    await waitForUrl(cdp, sessionId, url)
    const artifacts = join(dir, 'neo')
    const scriptPath = join(dir, 'neo.js')
    mkdirSync(dir, { recursive: true })
    writeFileSync(
      scriptPath,
      `${DECODERS}\nconst at = (name) => name;\n${source}`,
    )
    const neo = startNeo(neoRun, port, targetId, scriptPath, [
      '--artifacts',
      artifacts,
    ])
    await neo.connected
    const outcome = await neo.outcome
    const result =
      outcome.status === 'ok'
        ? { value: outcome.value }
        : {
            error: {
              name: outcome.error?.name,
              strict: /strict mode violation/.test(
                outcome.error?.message ?? '',
              ),
            },
          }
    let after
    if (probe) {
      const reply = await cdp.send(
        'Runtime.evaluate',
        // A probe may answer a promise, as Playwright's `page.evaluate` awaits.
        { expression: probe, returnByValue: true, awaitPromise: true },
        sessionId,
      )
      after = reply.result.value
    }
    return {
      ...result,
      probe: after,
      outcome,
      file: (name) => existing(join(artifacts, name)),
    }
  } finally {
    await cdp.send('Target.closeTarget', { targetId })
  }
}

function existing(path) {
  return existsSync(path) ? path : null
}
