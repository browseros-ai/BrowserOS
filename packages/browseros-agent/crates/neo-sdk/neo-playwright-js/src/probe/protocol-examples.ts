/**
 * Typed examples of every probe request and reply: the protocol's executable contract.
 * scripts/build.ts writes them to crates/neo-sdk/neo-playwright/tests/data/probe-protocol.json,
 * which Rust decodes, and tests/probe-protocol.test.ts sends every request through
 * the built asset. The mapped types make a request op or a reply kind without an example a
 * type error, so a new kind cannot ship undocumented on either side.
 */
import { parseSelector } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { ProbeCall, ProbeReply, ProbeTarget } from './generated'
import {
  CALL_FUNCTION,
  type Op,
  PROTOCOL,
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
  privateShown: [{ op: 'privateShown' }],
  webStorage: [
    { op: 'webStorage', kind: 'local', action: 'items' },
    { op: 'webStorage', kind: 'session', action: 'getItem', name: 'token' },
    {
      op: 'webStorage',
      kind: 'local',
      action: 'setItem',
      name: 'token',
      value: 'abc',
    },
    { op: 'webStorage', kind: 'local', action: 'removeItem', name: 'token' },
    { op: 'webStorage', kind: 'session', action: 'clear' },
  ],
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
      scroll: 'protocol',
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
        browserMapped: true,
      },
    },
  ],
  verdict: [{ op: 'verdict', token: 11 }],
  moves: [{ op: 'moves' }],
  disarm: [{ op: 'disarm' }],
  frameBox: [
    { op: 'frameBox', iframe: chunk('aria-ref=#1'), look: false },
    { op: 'frameBox', iframe: { pin: 4 }, point: { x: 40.5, y: 12 }, look: true },
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
    { op: 'edit', pin: 3, edit: { kind: 'focusToType' } },
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
    {
      op: 'edit',
      pin: 3,
      edit: {
        kind: 'drop',
        payloads: [],
        data: [{ mimeType: 'text/plain', value: 'external' }],
        point: { x: 20, y: 30 },
      },
    },
  ],
  look: [
    {
      op: 'look',
      pin: 3,
      look: {
        expect: { origin: 'https://pay.example' },
        length: 16,
        cut: false,
        keys: false,
        frameVisible: true,
      },
    },
    {
      op: 'look',
      pin: 3,
      look: {
        expect: {
          origin: 'https://bank.example',
          type: 'password',
          autocomplete: 'current-password',
          maxLength: 8,
          inputMode: '',
          name: 'password',
          id: 'pw',
        },
        cut: true,
        keys: false,
        frameVisible: true,
      },
    },
    {
      op: 'look',
      pin: 5,
      look: { cut: false, keys: true, frameVisible: true },
    },
  ],
  private: [
    {
      op: 'private',
      pin: 3,
      step: { kind: 'focus', text: 'CANARY-example', spaced: false },
    },
    { op: 'private', pin: 3, step: { kind: 'focused' } },
    {
      op: 'private',
      pin: 3,
      step: { kind: 'holds', text: '4242 4242', spaced: true },
    },
    { op: 'private', pin: 3, step: { kind: 'readBack' } },
    { op: 'private', pin: 3, step: { kind: 'forget' } },
    { op: 'private', pin: 3, step: { kind: 'markFocused' } },
    {
      op: 'private',
      pin: 6,
      step: { kind: 'drawn', text: 'CANARY-example' },
    },
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
      read: { kind: 'ref' },
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
      target: { pin: 3 },
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'viewCentre' },
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
    {
      op: 'snapshot',
      root: 'document',
      mode: 'ai',
    },
    { op: 'snapshot', root: chunk('main'), mode: 'default', depth: 3 },
    { op: 'snapshot', root: 'document', mode: 'default', boxes: true },
    { op: 'snapshot', root: 'document', mode: 'ai', closedRoots: 2 },
    { op: 'snapshot', root: 'document', mode: 'ai' },
    {
      op: 'snapshot',
      root: chunk('main'),
      mode: 'default',
    },
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
    { op: 'document', part: 'content', includeShadow: true },
    { op: 'document', part: 'expectSnapshot' },
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
    {
      kind: 'element',
      pin: 3,
      preview: '<button>Go</button>',
      resolved: { preview: '<button>Go</button>', count: 2 },
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
  notFocused: [
    { kind: 'notFocused', holder: '<input id="user" aria-label="Email"/>' },
    { kind: 'notFocused' },
  ],
  state: [
    { kind: 'state', missing: 'visible' },
    {
      kind: 'state',
      missing: 'enabled',
      resolved: { preview: '<button disabled>Go</button>', count: 1 },
    },
  ],
  measured: [
    {
      kind: 'measured',
      pin: 3,
      rect: RECT,
      point: POINT,
      inView: true,
      transformed: false,
      borderOffset: { x: 5, y: 5 },
    },
    {
      kind: 'measured',
      pin: 3,
      rect: RECT,
      point: POINT,
      inView: true,
      transformed: false,
      resolved: { preview: '<button>Go</button>', count: 1 },
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
    { kind: 'frameBox', origin: { x: 8, y: 120 }, hit: null, unseen: false },
    { kind: 'frameBox', origin: null, hit: 'done', unseen: false },
    {
      kind: 'frameBox',
      origin: { x: 8, y: 120 },
      hit: { hitTargetDescription: '<html></html>' },
      unseen: true,
    },
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
    { kind: 'value', value: [{ name: 'token', value: 'abc' }] },
    { kind: 'value', value: true },
    {
      kind: 'value',
      value: {
        facts: {
          origin: 'https://pay.example',
          type: 'text',
          autocomplete: 'cc-number',
          maxLength: 19,
          inputMode: 'numeric',
          name: 'cardnumber',
          id: '',
          sized: true,
          empty: true,
        },
      },
    },
    {
      kind: 'value',
      value: {
        facts: {
          origin: 'https://bank.example',
          type: 'password',
          autocomplete: 'current-password',
          maxLength: null,
          inputMode: '',
          name: 'password',
          id: 'pw',
          sized: true,
          empty: true,
        },
        refusal: { code: 'boxCovered' },
      },
    },
    {
      kind: 'value',
      value: { refusal: { code: 'notAnInput' } },
    },
    { kind: 'value', value: { kind: 'focus', focus: 'held' } },
    { kind: 'value', value: { kind: 'focus', focus: 'noDocumentFocus' } },
    { kind: 'value', value: { kind: 'focus', focus: 'moving' } },
    { kind: 'value', value: { kind: 'holds', holds: true } },
    {
      kind: 'value',
      value: {
        kind: 'readBack',
        state: 'left',
        heldAtLastInput: true,
        moving: false,
        drawing: 'none',
      },
    },
    {
      kind: 'value',
      value: {
        kind: 'readBack',
        state: 'holds',
        heldAtLastInput: true,
        moving: false,
        drawing: 'drawn',
      },
    },
    { kind: 'value', value: { kind: 'forgot' } },
    { kind: 'value', value: { kind: 'markedFocused', marked: true } },
    { kind: 'value', value: { kind: 'drawn', drawn: false } },
  ],
  read: [
    { kind: 'read', value: 'Welcome' },
    {
      kind: 'read',
      value: { multiple: false, webkitdirectory: false },
      resolved: { preview: '<input type="file"/>', count: 1 },
    },
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
    {
      kind: 'snapshot',
      json: [{ role: 'generic', ref: 'e1', text: 'Open menu' }],
      iframes: [],
      marksSkipped: true,
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
    {
      kind: 'expect',
      matches: false,
      resolved: '<h1 id="greeting">Hello</h1>',
      received: 'Hello',
      ariaSnapshot: '- heading "Hello" [level=1]',
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
    {
      kind: 'error',
      class: 'script',
      message: 'Node is not an HTMLInputElement',
      resolved: {
        preview: '<div id="notfile">Not a file input</div>',
        count: 1,
      },
    },
    { kind: 'error', class: 'internal', message: 'not built yet' },
  ],
  uninstalled: [{ kind: 'uninstalled' }],
}

/** Both policies belong to the fingerprinted ABI, alongside the request variants. */
const CALL_EXAMPLES: AtLeastOne<ProbeCall> = [
  {
    request: REQUEST_EXAMPLES.snapshot[0],
    policy: { conformance: false, mayHaveOtherFrame: true },
  },
  {
    request: REQUEST_EXAMPLES.expect[0],
    policy: { conformance: true, mayHaveOtherFrame: false },
  },
]

/** The document scripts/build.ts writes to probe-protocol.json. */
export function protocolDocument() {
  return {
    protocol: PROTOCOL,
    callFunction: CALL_FUNCTION,
    replyKinds: REPLY_KINDS,
    byHandle: ['pinned'],
    anyRequest: ['error', 'uninstalled'],
    calls: CALL_EXAMPLES,
    requests: Object.values(REQUEST_EXAMPLES).flat(),
    replies: Object.values(REPLY_EXAMPLES).flat(),
  }
}
