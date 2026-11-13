// Clickable marking's container guard (src/probe/pointer-targets.ts `leafTargets`): a target
// that holds a target of its own is never passed to the engine as a pointer target, because the
// engine folds everything inside one into it and marking must never take a ref or a mark away.
// Only what the engine draws counts. The tag and role lists are desktop Neo's own, pinned here to
// browseros-core's source.
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
  <div id="none" tabindex="0">Open menu<button style="display: none">Hidden</button></div>
  <div id="drop" tabindex="0">Drop files here<input type="file" hidden></div>
  <div id="zero" tabindex="0"><input type="checkbox" style="width: 0; height: 0"><span>Notifications</span></div>
  <div id="hidden-nav" tabindex="0">Docs<nav style="display: none"><a href="#x">Docs</a></nav></div>
  <div id="unseen" tabindex="0">Menu<span style="visibility: hidden"><button>Close</button></span></div>
  <div id="row" tabindex="0">Row text<div id="row-hit" style="visibility: hidden">Hidden hit</div></div>
  <div id="seen" tabindex="0"><span style="visibility: hidden"><button style="visibility: visible">Open</button></span></div>
  <div id="drawn" tabindex="0">Card text<button aria-hidden="true" tabindex="-1">x</button></div>
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
// A stand-in for the engine's visibility test (InjectedScript.utils.isElementVisible, which
// prelude_live exercises on a real page) from inline styles, as these pages need it: not drawn
// under `display: none` or `hidden`, with a zero width, or when the nearest `visibility` is
// `hidden`. linkedom lays nothing out.
const isVisible = (element: Element): boolean => {
  const style = (of: Element) => of.getAttribute('style') ?? ''
  let visibility: string | undefined
  for (let node: Element | null = element; node; node = node.parentElement) {
    if (node.hasAttribute('hidden') || /display:\s*none/.test(style(node)))
      return false
    visibility ??= /visibility:\s*(\w+)/.exec(style(node))?.[1]
  }
  return visibility !== 'hidden' && !/width:\s*0/.test(style(element))
}
const leaves = (targets: Element[]) =>
  leafTargets(targets, { roleOf, isVisible })

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

it('tests that a control the engine does not draw does not make a container, and a drawn one does', () => {
  // display: none, the `hidden` attribute, a zero-size box, a hidden section, a control under
  // `visibility: hidden`, and a hidden target of its own: none prints with a ref, so each wrapper
  // is the only thing to click.
  const hidden = [
    'none',
    'drop',
    'zero',
    'hidden-nav',
    'unseen',
    'row',
    'row-hit',
  ]
  expect(leaves(hidden.map(byId)).map((target) => target.id)).toEqual(hidden)
  // A child drawn under a hidden parent still counts, as does a drawn `aria-hidden` control: in
  // ai mode the engine prints it with its ref.
  expect(leaves(['seen', 'drawn'].map(byId))).toEqual([])
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
