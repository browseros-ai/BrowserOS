/**
 * The agent-runtime bundle, crates/browseros-playwright/assets/prelude.js: Playwright's client
 * API for the scripts agents send to Neo's `run` tool.
 *
 * claw-sdk evaluates this IIFE in each run's fresh QuickJS realm, after its bootstrap and
 * before the agent's function, so every object here lives for one run. It defines no global:
 * it gives the agent's function four parameters through `__neo.provide`, the names
 * Playwright examples use: `page` (the run's own tab), `context`, `browser` and `expect`.
 * Every call they make reaches Rust as one op through bridge.ts, and every selector is built
 * by the vendored Playwright helpers imported in vendored.ts, in this one module graph.
 * QuickJS has no URL, Intl or structuredClone; tests/prelude-globals.test.ts fails if the
 * bundle names any global outside ECMAScript's own.
 */

import { onReply, provide, registerBrand } from './bridge'
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
onReply((reply) => context._applyReply(reply))

// The agent function's parameters, after the bootstrap's own. claw-sdk's placeholder page
// prelude provides `page` and `context` too; these replace them when both are loaded.
provide('page', (init) => context._runPage(init))
provide('context', () => context)
provide('browser', () => browser)
provide('expect', () => expect)
