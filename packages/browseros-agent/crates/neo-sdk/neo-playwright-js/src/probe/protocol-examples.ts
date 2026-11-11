/**
 * Typed examples of every probe request and reply: the protocol's executable contract.
 * scripts/build.ts writes them to crates/neo-sdk/neo-playwright/tests/data/probe-protocol.json,
 * which the Rust mirror decodes, and tests/probe-protocol.test.ts sends every request through
 * the built asset. The mapped types make a request op or a reply kind without an example a
 * type error, so a new kind cannot ship undocumented on either side.
 */
import { parseSelector } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import {
  CALL_FUNCTION,
  type Op,
  PROTOCOL,
  type ProbeReply,
  type ProbeTarget,
  REPLY_KINDS,
  type ReplyKind,
  type RequestOf,
} from './protocol'

export type AtLeastOne<T> = readonly [T, ...T[]]

/** A target as the prelude sends it: the vendored parser's output, never a string. */
const chunk = (selector: string): ProbeTarget => ({
  chunk: parseSelector(selector),
})

const GO_BUTTON = chunk('internal:role=button[name="Go"i]')

export const REQUEST_EXAMPLES: { [O in Op]: AtLeastOne<RequestOf<O>> } = {
  describe: [{ op: 'describe' }],
  screenshotMask: [{ op: 'screenshotMask', target: GO_BUTTON, color: '#F0F' }],
  screenshotMaskCleanup: [{ op: 'screenshotMaskCleanup' }],
  resolve: [
    { op: 'resolve', target: GO_BUTTON, strictness: 'strict', want: 'element' },
    {
      op: 'resolve',
      target: chunk('#pay'),
      strictness: 'strict',
      want: 'frame',
    },
    {
      op: 'resolve',
      target: chunk('internal:role=listitem >> internal:has="aria-ref=e7"'),
      strictness: 'firstMatch',
      want: 'element',
    },
    {
      op: 'resolve',
      target: chunk('ref=e5'),
      strictness: 'strict',
      want: 'element',
    },
    {
      op: 'resolve',
      target: { pin: 3 },
      strictness: 'strict',
      want: 'element',
    },
    {
      op: 'resolve',
      target: 'document',
      strictness: 'strict',
      want: 'element',
    },
  ],
  act: [
    {
      op: 'act',
      target: GO_BUTTON,
      strictness: 'strict',
      retarget: 'button-link',
      states: ['visible', 'enabled'],
      frameVisible: true,
      scroll: 'ifNeeded',
    },
    {
      op: 'act',
      target: { pin: 3 },
      strictness: 'strict',
      retarget: 'none',
      states: [],
      frameVisible: true,
      scroll: 'none',
      position: { x: 5, y: 5 },
      arm: {
        action: 'mouse',
        blockAll: false,
        token: 11,
        expiryMs: 8000,
        expectRect: { x: 8, y: 8, width: 60, height: 20 },
      },
    },
  ],
  verdict: [{ op: 'verdict', token: 11 }],
  moves: [{ op: 'moves' }],
  disarm: [{ op: 'disarm' }],
  frameBox: [
    { op: 'frameBox', iframe: chunk('aria-ref=#1') },
    { op: 'frameBox', iframe: { pin: 4 }, point: { x: 40.5, y: 12 } },
  ],
  edit: [
    { op: 'edit', pin: 3, edit: { kind: 'fill', value: 'hello' } },
    {
      op: 'edit',
      pin: 3,
      edit: {
        kind: 'selectOptions',
        options: [
          { valueOrLabel: 'Blue' },
          { value: 'r' },
          { label: 'Green' },
          { index: 2 },
        ],
      },
    },
    { op: 'edit', pin: 3, edit: { kind: 'selectText' } },
    { op: 'edit', pin: 3, edit: { kind: 'focus', resetSelection: true } },
    { op: 'edit', pin: 3, edit: { kind: 'blur' } },
    {
      op: 'edit',
      pin: 3,
      edit: { kind: 'dispatchEvent', type: 'click', init: { bubbles: true } },
    },
    {
      op: 'edit',
      pin: 3,
      edit: {
        kind: 'setFiles',
        payloads: [
          { name: 'note.txt', mimeType: 'text/plain', buffer: 'aGVsbG8=' },
        ],
      },
    },
    { op: 'edit', pin: 3, edit: { kind: 'setFiles', payloads: [] } },
  ],
  read: [
    {
      op: 'read',
      target: GO_BUTTON,
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'textContent' },
    },
    {
      op: 'read',
      target: chunk('#pw'),
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'inputValue' },
    },
    {
      op: 'read',
      target: GO_BUTTON,
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'attribute', name: 'aria-label' },
    },
    {
      op: 'read',
      target: GO_BUTTON,
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'state', state: 'checked' },
    },
    {
      op: 'read',
      target: { pin: 3 },
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'checkable' },
    },
    {
      op: 'read',
      target: 'root',
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'fileInput', multiple: false, directory: false },
    },
    {
      op: 'read',
      target: chunk('#upload'),
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'fileInput', multiple: true, directory: true },
    },
    {
      op: 'read',
      target: { pin: 3 },
      strictness: 'strict',
      frameVisible: false,
      read: { kind: 'boundingBox' },
    },
    {
      op: 'read',
      target: chunk('main'),
      strictness: 'firstMatch',
      frameVisible: true,
      read: { kind: 'innerText' },
    },
    {
      op: 'read',
      target: chunk('main'),
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'innerHtml' },
    },
    {
      op: 'read',
      target: chunk('#pay'),
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'frameOrigin' },
    },
    {
      op: 'read',
      target: chunk('li'),
      strictness: 'firstMatch',
      frameVisible: true,
      read: { kind: 'located' },
    },
    {
      op: 'read',
      target: 'root',
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'state', state: 'visible' },
    },
  ],
  readAll: [
    { op: 'readAll', target: chunk('internal:role=listitem'), read: 'count' },
    { op: 'readAll', target: chunk('li'), read: 'textContents' },
    { op: 'readAll', target: chunk('li'), read: 'innerTexts' },
    { op: 'readAll', target: chunk('li'), read: 'pins' },
  ],
  selector: [
    { op: 'selector', target: chunk('internal:role=button') },
    { op: 'selector', target: { pin: 4 } },
  ],
  snapshot: [
    { op: 'snapshot', root: 'document', mode: 'ai' },
    { op: 'snapshot', root: chunk('main'), mode: 'default', depth: 3 },
    { op: 'snapshot', root: 'document', mode: 'default', boxes: true },
  ],
  expect: [
    {
      op: 'expect',
      target: chunk('#greeting'),
      strictness: 'strict',
      frameVisible: true,
      params: {
        expression: 'to.have.text',
        expectedText: [
          {
            string: 'Welcome',
            matchSubstring: false,
            normalizeWhiteSpace: true,
          },
        ],
        isNot: false,
        useInnerText: false,
      },
    },
    {
      op: 'expect',
      target: chunk('internal:role=listitem'),
      strictness: 'firstMatch',
      frameVisible: true,
      params: { expression: 'to.have.count', expectedNumber: 3, isNot: true },
    },
    {
      op: 'expect',
      target: 'document',
      strictness: 'strict',
      frameVisible: true,
      params: {
        expression: 'to.have.title',
        expectedText: [
          { regexSource: 'done$', regexFlags: 'i', normalizeWhiteSpace: true },
        ],
        isNot: false,
      },
    },
  ],
  matchUrl: [
    {
      op: 'matchUrl',
      pattern: { glob: '**/done' },
      url: 'https://example.com/done',
    },
    {
      op: 'matchUrl',
      pattern: { regexSource: 'done$', regexFlags: 'i' },
      url: 'https://example.com/DONE',
    },
  ],
  pinned: [{ op: 'pinned', pin: 4 }],
  document: [
    { op: 'document', part: 'title' },
    { op: 'document', part: 'content' },
  ],
  armInput: [{ op: 'armInput', pin: 4 }],
  inputArrived: [{ op: 'inputArrived', pin: 4 }],
  // Last: it replaces the document, so the protocol test's later requests would find none of
  // the elements they name.
  setContent: [
    { op: 'setContent', html: '<!doctype html><title>Set</title><p>new</p>' },
  ],
}

const RECT = { x: 8, y: 8, width: 60, height: 20 }
const POINT = { x: 38, y: 18 }

const REPLY_EXAMPLES: {
  [K in ReplyKind]: AtLeastOne<Extract<ProbeReply, { kind: K }>>
} = {
  described: [
    {
      kind: 'described',
      protocol: PROTOCOL,
      seq: 0,
      url: 'https://example.com/',
    },
    {
      kind: 'described',
      protocol: PROTOCOL,
      seq: 347,
      url: 'https://pay.example/frame',
    },
    {
      kind: 'described',
      protocol: PROTOCOL,
      seq: 512340,
      url: 'https://example.com/',
      retired: 0,
    },
  ],
  none: [{ kind: 'none' }, { kind: 'none', staleRef: 'f2e5' }],
  strict: [
    {
      kind: 'strict',
      message:
        "strict mode violation: getByRole('button') resolved to 2 elements:\n    1) <button>Go</button> aka getByRole('button', { name: 'Go' })\n    2) <button>Stop</button> aka getByRole('button', { name: 'Stop' })\n",
    },
  ],
  element: [
    {
      kind: 'element',
      pin: 3,
      preview: '<button>Go</button>',
    },
  ],
  frame: [
    {
      kind: 'frame',
      pin: 4,
      key: '#1',
      visible: true,
      src: 'https://pay.example/frame',
    },
  ],
  notFrame: [{ kind: 'notFrame', preview: '<div id="pay">…</div>' }],
  detached: [{ kind: 'detached' }],
  state: [{ kind: 'state', missing: 'visible' }],
  measured: [
    {
      kind: 'measured',
      pin: 3,
      rect: RECT,
      point: POINT,
      inView: true,
      transformed: false,
    },
  ],
  covered: [
    {
      kind: 'covered',
      by: '<div id="consent">…</div> intercepts pointer events',
    },
  ],
  armed: [{ kind: 'armed', rect: RECT, point: POINT, token: 11 }],
  verdict: [
    { kind: 'verdict', hit: 'done' },
    {
      kind: 'verdict',
      hit: { hitTargetDescription: '<div id="consent">…</div>' },
    },
  ],
  moves: [{ kind: 'moves', moves: 3 }],
  frameBox: [
    { kind: 'frameBox', origin: { x: 8, y: 120 }, hit: null },
    { kind: 'frameBox', origin: null, hit: 'done' },
  ],
  value: [
    { kind: 'value', value: 'Welcome' },
    { kind: 'value', value: 'Ada' },
    { kind: 'value', value: null },
    { kind: 'value', value: RECT },
    { kind: 'value', value: ['r', 'g'] },
    { kind: 'value', value: 'needsinput' },
    { kind: 'value', value: 'error:optionsnotfound' },
    { kind: 'value', value: { threw: 'DOMException' } },
    { kind: 'value', value: true },
  ],
  read: [
    { kind: 'read', value: 'Welcome' },
    { kind: 'read', value: null },
    { kind: 'read', value: RECT },
    {
      kind: 'read',
      value: { x: 8, y: 120, scaleX: 1, scaleY: 1 },
    },
    { kind: 'read', value: 'rotated' },
    {
      kind: 'read',
      value: { checked: true, radio: true },
    },
    {
      kind: 'read',
      value: { pin: 7, target: 8, takesPaths: true },
    },
    {
      kind: 'read',
      value: { pin: 6, preview: '<li>One</li>', visible: true, count: 2 },
    },
  ],
  values: [
    { kind: 'values', count: 2, values: ['One', 'Two'] },
    { kind: 'values', count: 2, values: [] },
    {
      kind: 'values',
      count: 2,
      values: [
        { pin: 6, preview: '<li>One</li>' },
        { pin: 7, preview: '<li>Two</li>' },
      ],
    },
  ],
  selector: [
    { kind: 'selector', selector: 'internal:role=button[name="Go"i]' },
    { kind: 'selector', selector: 'iframe[name="pay"]' },
  ],
  snapshot: [
    {
      kind: 'snapshot',
      json: [
        { role: 'heading', name: 'Pay', level: 1 },
        { role: 'button', name: 'Go', ref: 'e2', cursor: 'pointer' },
        { role: 'iframe', ref: 'e3' },
      ],
      iframes: [{ ref: 'e3', depth: 1, src: 'https://pay.example/frame' }],
    },
  ],
  expect: [
    {
      kind: 'expect',
      matches: true,
      resolved: '<h1 id="greeting">Hello</h1>',
      received: 'Hello',
    },
    {
      kind: 'expect',
      matches: false,
      resolved: '3 elements',
      received: ['one', 'two', 'three'],
    },
    {
      kind: 'expect',
      matches: false,
      resolved: '<input id="name"/>',
      received: null,
    },
    {
      kind: 'expect',
      matches: false,
      resolved: '<div id="x">…</div>',
    },
  ],
  urlMatch: [
    { kind: 'urlMatch', matches: true, url: 'https://example.com/done' },
  ],
  documentText: [
    { kind: 'documentText', text: 'Pay' },
    {
      kind: 'documentText',
      text: '<!DOCTYPE html><html><head></head><body><input type="password" value="hunter2"></body></html>',
    },
  ],
  replaced: [{ kind: 'replaced' }],
  error: [
    {
      kind: 'error',
      class: 'selector',
      message:
        "Unknown selector engine \"ref\". Use page.getByRef('e5') or 'aria-ref=e5'.",
    },
    {
      kind: 'error',
      class: 'script',
      message: 'Not a checkbox or radio button',
    },
    {
      kind: 'error',
      class: 'unsupported',
      message:
        'toHaveValue(RegExp) is not supported on password, card and one-time-code fields; use an exact string.',
    },
    { kind: 'error', class: 'internal', message: 'not built yet' },
  ],
  uninstalled: [{ kind: 'uninstalled' }],
}

/** The document scripts/build.ts writes to probe-protocol.json. */
export function protocolDocument() {
  return {
    protocol: PROTOCOL,
    callFunction: CALL_FUNCTION,
    replyKinds: REPLY_KINDS,
    byHandle: ['pinned'],
    anyRequest: ['error', 'uninstalled'],
    requests: Object.values(REQUEST_EXAMPLES).flat(),
    replies: Object.values(REPLY_EXAMPLES).flat(),
  }
}
