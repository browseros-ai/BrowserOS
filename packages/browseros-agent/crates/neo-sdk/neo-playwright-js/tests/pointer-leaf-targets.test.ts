// Clickable marking's container guard (src/probe/pointer-targets.ts `leafTargets`): a target
// that holds a target of its own is never passed to the engine as a pointer target, because the
// engine folds everything inside one into it and marking must never take a ref or a mark away.
// The tag and role lists are desktop Neo's own, pinned here to browseros-core's source.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { parseHTML } from 'linkedom'
import {
  INTERACTIVE_ROLES,
  INTERACTIVE_TAGS,
  leafTargets,
  SECTION_ROLES,
} from '../src/probe/pointer-targets'

const { document } = parseHTML(`<html><body>
  <div id="pane" tabindex="0"><section><p>Intro</p><a href="/x">Docs</a></section></div>
  <div id="panel" role="tabpanel" tabindex="0"><span role="button">Next</span></div>
  <div id="card" tabindex="0"><span id="pointer" style="cursor: pointer"></span></div>
  <div id="host" tabindex="0"></div>
  <div id="icon"><img alt="Settings"></div>
  <div id="menu">Open <b>menu</b></div>
  <div id="tile"></div>
  <ul id="list" tabindex="0"><li><div id="item-a">A</div></li><li><div id="item-b">B</div></li></ul>
  <div id="svg-box" tabindex="0"><svg><slot></slot></svg></div>
  <div id="story" tabindex="0"><article><p>Only text</p></article></div>
  <div id="region" tabindex="0"><div role="region" aria-label="Notes">Only text</div></div>
  <div id="framed" tabindex="0"><iframe title="pay"></iframe></div>
  <div id="tokens" tabindex="0"><div role="region group" aria-label="Notes">Only text</div></div>
  <div id="spaced" tabindex="0"><div role="region " aria-label="Notes">Only text</div></div>
  <div id="unnamed" tabindex="0"><section><p>Plain</p></section></div>
</body></html>`)
// linkedom computes no styles: read an inline cursor, as Chromium would compute it here.
Reflect.set(globalThis, 'getComputedStyle', (element: Element) => ({
  cursor: /cursor:\s*pointer/.test(element.getAttribute('style') ?? '')
    ? 'pointer'
    : 'auto',
}))
const byId = (id: string) => {
  const found = document.getElementById(id)
  if (!found) throw new Error(`no #${id}`)
  return found
}
const host = byId('host')
const shadow = host.attachShadow({ mode: 'open' })
shadow.innerHTML = '<button>Inside</button>'

// A stand-in for the engine's role reading (InjectedScript.utils.getAriaRole, which the probe
// passes and prelude_live exercises on a real page): the first known role token, else the few
// implicit roles these pages use. The vendored module's `@isomorphic` imports resolve only in
// the asset build.
const KNOWN_ROLES = new Set([
  ...INTERACTIVE_ROLES,
  ...SECTION_ROLES,
  'group',
  'tabpanel',
  'list',
  'listitem',
])
const IMPLICIT: Record<string, string> = {
  article: 'article',
  a: 'link',
  button: 'button',
  li: 'listitem',
  ul: 'list',
}
const roleOf = (element: Element): string | null =>
  (element.getAttribute('role') ?? '')
    .split(' ')
    .map((token) => token.trim())
    .find((token) => KNOWN_ROLES.has(token)) ??
  IMPLICIT[element.localName] ??
  null
const leaves = (targets: Element[]) => leafTargets(targets, roleOf)

it('tests that a target holding a control, a pointer element or another target is left out, and leaves are kept', () => {
  const ids = ['pane', 'panel', 'card', 'host', 'icon', 'menu', 'tile']
  expect(leaves(ids.map(byId)).map((target) => target.id)).toEqual([
    'icon',
    'menu',
    'tile',
  ])
  // A section prints its line, and its ref, only with its text: inside an actionable target
  // the v1 walk leaves that text out, so a target holding one is left out too, as is a frame's.
  // The role is the engine's reading: its first valid token, whatever the spacing.
  expect(
    leaves(['story', 'region', 'framed', 'tokens', 'spaced'].map(byId)),
  ).toEqual([])
  // An SVG `<slot>` is no HTML slot: walked as a plain element, and the box stays a leaf; so is
  // a box around an unnamed `<section>`, which the engine gives no region role.
  expect(
    leaves([byId('svg-box'), byId('unnamed')]).map((target) => target.id),
  ).toEqual(['svg-box', 'unnamed'])
  // A tabindex list of clickable divs: the list folds the same way, its items are leaves.
  const list = ['list', 'item-a', 'item-b'].map(byId)
  expect(leaves(list).map((target) => target.id)).toEqual(['item-a', 'item-b'])
})

it("tests that the interactive tags and roles are desktop Neo's own", () => {
  const source = readFileSync(
    new URL(
      '../../../browseros-core/src/observer/clickable.rs',
      import.meta.url,
    ),
    'utf8',
  )
  const list = (name: string) => {
    const body = source.match(
      new RegExp(`const ${name}: &\\[&str\\] = &\\[([^\\]]*)\\]`),
    )?.[1]
    if (body === undefined) throw new Error(`clickable.rs has no ${name}`)
    return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1])
  }
  expect([...INTERACTIVE_TAGS]).toEqual(list('INTERACTIVE_TAGS'))
  expect([...INTERACTIVE_ROLES]).toEqual(list('INTERACTIVE_ROLES'))
})

it("tests that the section roles are the v1 renderer's, its iframe aside", () => {
  const source = readFileSync(
    new URL('../src/prelude/snapshot-v1.ts', import.meta.url),
    'utf8',
  )
  const body = source.match(
    /const SECTION = new Set<string>\(\[([^\]]*)\]\)/,
  )?.[1]
  if (body === undefined) throw new Error('snapshot-v1.ts has no SECTION')
  const roles = [...body.matchAll(/'([^']+)'/g)].map((match) => match[1])
  expect([...SECTION_ROLES]).toEqual(roles.filter((role) => role !== 'iframe'))
})
