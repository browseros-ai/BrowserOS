/**
 * The agent-runtime bundle, crates/neo-sdk/neo-playwright/assets/prelude.js: Playwright's client
 * API for the scripts agents send to Neo's `run` tool.
 *
 * claw-sdk evaluates this IIFE in each run's fresh QuickJS realm, after its bootstrap and
 * before the agent's function, so every object here lives for one run. It defines no global:
 * it gives the agent's function five parameters through `__neo.provide`, the names
 * Playwright examples use: `page` (the run's own tab), `context`, `browser`, `expect`, and
 * Node's `Buffer` (buffer.ts), which upstream's payload examples build files with.
 * Every call they make reaches Rust as one op through bridge.ts, and every selector is built
 * by the vendored Playwright helpers imported in vendored.ts, in this one module graph.
 * neo-sdk installs URL and base64 globals; QuickJS has no Intl or structuredClone.
 * tests/prelude-globals.test.ts pins the globals this bundle may name.
 */

import { awaitsProviders, onReply, provide, registerBrand } from './bridge'
import { Browser } from './browser'
import { Buffer } from './buffer'
import { BrowserContext } from './context'
import { ElementHandle, elementBrand } from './element-handle'
import { ConsoleMessage, Download } from './event-values'
import { createExpect } from './expect'
import { Frame } from './frame'
import { Keyboard, Mouse } from './input'
import { FrameLocator, Locator } from './locator'
import { Page } from './page'
import { Response } from './response'
import { installUnsupported } from './unsupported'

installUnsupported({
  Browser,
  BrowserContext,
  Page,
  Frame,
  Locator,
  FrameLocator,
  Keyboard,
  Mouse,
  ElementHandle,
  Download,
  ConsoleMessage,
  Response,
})

const context = new BrowserContext()
const browser = new Browser(context)
const expect = createExpect()

// Page and element handles in op results come back as Page and ElementHandle objects, one
// Page per page id.
registerBrand('page', (json) => context._pageFromResult(json))
registerBrand('element', (json) => {
  const brand = elementBrand(json)
  return brand ? new ElementHandle(context._pageFor(brand.page), brand) : json
})
// Screenshots and PDFs resolve to a Buffer, as upstream's do, so `toString('base64')` is
// base64 and the bytes can be a `setInputFiles` payload's buffer.
registerBrand('bytes', (json) =>
  Buffer.from(String(Reflect.get(json, 'base64') ?? ''), 'base64'),
)
onReply((reply) => context._applyReply(reply))

// The agent function's parameters, after the bootstrap's own. claw-sdk's placeholder page
// prelude provides `page` and `context` too; these replace them when both are loaded.
// With a bootstrap that awaits its providers (FR/RC C5), the run's page reads its frame tree
// before the agent's first statement, so `page.frames()` lists the tab's iframes from the start,
// as upstream's does (FR-1). An older bootstrap would hand the script the promise instead.
provide(
  'page',
  awaitsProviders()
    ? async (init) => {
        const page = context._runPage(init)
        await page._readInitialFrames()
        return page
      }
    : (init) => context._runPage(init),
)
provide('context', () => context)
provide('browser', () => browser)
provide('expect', () => expect)
provide('Buffer', () => Buffer)
