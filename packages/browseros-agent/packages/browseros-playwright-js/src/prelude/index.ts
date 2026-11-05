/**
 * The agent-runtime bundle, crates/browseros-playwright/assets/prelude.js: Playwright's client
 * API for the scripts agents send to Neo's `run` tool.
 *
 * claw-sdk evaluates this IIFE in each run's fresh QuickJS runtime, after its bootstrap and
 * before the script, so every object here lives for one run. It defines four globals, the
 * names Playwright examples use: `page` (the run's default tab), `context`, `browser` and
 * `expect`. Every call they make reaches Rust as one op through bridge.ts, and every selector
 * is built by the vendored Playwright helpers imported in vendored.ts, in this one module
 * graph. QuickJS has no URL, Intl or structuredClone; tests/prelude-globals.test.ts fails if
 * the bundle names any global outside ECMAScript's own.
 */

import { onPageFacts, readSeed, registerBrand } from './bridge'
import { Browser } from './browser'
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

const seed = readSeed()
const context = new BrowserContext(seed)
const browser = new Browser(context, seed.browserVersion)

// Page and element handles in op results come back as Page and ElementHandle objects, one
// Page per page id, whether or not the bootstrap revives brands itself.
registerBrand('page', (json) => context._pageFromResult(json))
registerBrand('element', (json) => {
  const brand = elementBrand(json)
  return brand
    ? new ElementHandle(context._pageFor({ id: brand.page }), brand)
    : json
})
onPageFacts((facts) => context._applyFacts(facts))

Object.assign(globalThis, {
  page: context._defaultPage,
  context,
  browser,
  expect: createExpect(),
})
