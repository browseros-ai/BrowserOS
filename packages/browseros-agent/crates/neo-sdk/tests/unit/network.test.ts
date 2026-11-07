/**
 * `page.goto`'s `Response` and its `Request` chain against a stub `__neo_runtime`: how the SDK
 * builds them from Rust's `chain` getter, what each method sends and makes of the reply, and the
 * live client state read through getters. Each expectation is Playwright's client behaviour at
 * 26137face (`client/network.ts`, `client/frame.ts`, `client/channelOwner.ts`).
 */

import { beforeEach, describe, expect, test } from 'bun:test'
import { createRun } from '../../src/index'
import { Buffer as BodyBuffer, decodeUtf8 } from '../../src/network'
import { eventless } from './bridge-stub'

type Sent = { guid: string; method: string; params: unknown; timeout: number }
type Getter = { guid: string; field: string }

let sent: Sent[] = []
let getters: Getter[] = []
let replies: Record<string, unknown[]> = {}
let getterAnswers: Record<string, unknown[]> = {}

const GOTO = 'frame@T goto'
const FINAL = 'response@T:L:2'

function header(name: string, value: string) {
  return { name, value }
}

/** Two redirects, then the document: Rust's `chain` answer for the final response. */
function chain() {
  const request = (hop: number, url: string) => ({
    guid: `request@T:L:${hop}`,
    initializer: {
      frame: { guid: 'frame@T' },
      url,
      resourceType: 'document',
      method: 'GET',
      headers: [
        header('User-Agent', 'neo'),
        header('Accept', 'a'),
        header('accept', 'b'),
      ],
      isNavigationRequest: true,
      ...(hop ? { redirectedFrom: { guid: `request@T:L:${hop - 1}` } } : {}),
    },
  })
  const response = (
    hop: number,
    url: string,
    status: number,
    statusText: string,
  ) => ({
    guid: `response@T:L:${hop}`,
    initializer: {
      request: { guid: `request@T:L:${hop}` },
      url,
      status,
      statusText,
      headers: [
        header('Content-Type', 'text/html'),
        header('X-Dup', '2'),
        header('x-dup', '1'),
        header('Set-Cookie', 'a=1\nb=2'),
      ],
      timing: {
        startTime: 1000,
        domainLookupStart: -1,
        domainLookupEnd: -1,
        connectStart: -1,
        secureConnectionStart: -1,
        connectEnd: -1,
        requestStart: 0.1,
        responseStart: 0.5,
      },
      fromServiceWorker: false,
    },
  })
  return {
    requests: [
      request(0, 'http://a.test/r1'),
      request(1, 'http://a.test/r2'),
      request(2, 'http://a.test/ok'),
    ],
    responses: [
      response(0, 'http://a.test/r1', 302, 'Found'),
      response(1, 'http://a.test/r2', 301, 'Moved Permanently'),
      response(2, 'http://a.test/ok', 200, 'Fine'),
    ],
  }
}

beforeEach(() => {
  sent = []
  getters = []
  replies = { [GOTO]: [{ response: { guid: FINAL } }] }
  getterAnswers = { [`${FINAL} chain`]: [chain()] }
  ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
    get: (request: string) => {
      const { guid, field } = JSON.parse(request) as Getter
      getters.push({ guid, field })
      const answers = getterAnswers[`${guid} ${field}`]
      if (!answers?.length) throw new Error(`no answer for ${guid} ${field}`)
      return JSON.stringify(answers.length > 1 ? answers.shift() : answers[0])
    },
    call: async (message: string) => {
      const { id, guid, method, params, metadata } = JSON.parse(message)
      sent.push({ guid, method, params, timeout: metadata.timeout })
      const queue = replies[`${guid} ${method}`]
      const reply = queue?.length ? queue.shift() : undefined
      if (reply && typeof reply === 'object' && 'error' in reply)
        return JSON.stringify({ id, ...reply })
      return JSON.stringify(
        reply === undefined ? { id } : { id, result: reply },
      )
    },
  })
})

function run() {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@default',
    url: 'about:blank',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

async function gotoResponse() {
  const { page } = run()
  const response = await page.goto('http://a.test/r1')
  if (!response) throw new Error('goto resolved null')
  return { page, response }
}

describe('goto', () => {
  test('tests that goto resolves the final response of the redirect chain', async () => {
    const { page, response } = await gotoResponse()
    expect([
      response.url(),
      response.ok(),
      response.status(),
      response.statusText(),
    ]).toEqual(['http://a.test/ok', true, 200, 'Fine'])
    expect(response.fromServiceWorker()).toBe(false)
    expect(response.frame()).toBe(page.mainFrame())
    const request = response.request()
    expect([request.url(), request.method(), request.resourceType()]).toEqual([
      'http://a.test/ok',
      'GET',
      'document',
    ])
    expect(request.isNavigationRequest()).toBe(true)
    expect(request.frame()).toBe(page.mainFrame())
    expect(request.serviceWorker()).toBeNull()
    expect(request.existingResponse()).toBe(response)
    expect([
      request.postData(),
      request.postDataBuffer(),
      request.postDataJSON(),
    ]).toEqual([null, null, null])
    const first = request.redirectedFrom()?.redirectedFrom()
    expect(first?.url()).toBe('http://a.test/r1')
    expect(first?.redirectedFrom()).toBeNull()
    expect(first?.redirectedTo()?.redirectedTo()).toBe(request)
    expect(first?.existingResponse()?.status()).toBe(302)
    expect(getters).toEqual([{ guid: FINAL, field: 'chain' }])
    // The last hop knows of no later one until Rust's record says one started.
    getterAnswers[`${request._guid} redirectedTo`] = [null]
    expect(request.redirectedTo()).toBeNull()
    expect(getters.at(-1)).toEqual({
      guid: request._guid,
      field: 'redirectedTo',
    })
  })

  test('tests that goto resolves null when the navigation has no response', async () => {
    const { page } = run()
    replies[GOTO] = [{}]
    expect(await page.goto('about:blank')).toBeNull()
    expect(await page.mainFrame().goto('about:blank')).toBeNull()
    expect(getters).toEqual([])
  })

  test('tests that provisional headers read as Playwright merges them', async () => {
    const { response } = await gotoResponse()
    expect(response.headers()).toEqual({
      'content-type': 'text/html',
      'x-dup': '2, 1',
      'set-cookie': 'a=1\nb=2',
    })
    expect(response.request().headers()).toEqual({
      'user-agent': 'neo',
      accept: 'a, b',
    })
  })

  test('tests that responses and requests serialize as Playwright channel objects', async () => {
    const { response } = await gotoResponse()
    expect(JSON.parse(JSON.stringify(response))).toEqual({
      _type: 'Response',
      _guid: FINAL,
    })
    expect(JSON.parse(JSON.stringify(response.request()))).toEqual({
      _type: 'Request',
      _guid: 'request@T:L:2',
    })
  })
})

describe('Response', () => {
  test('tests that raw headers are read once and answered as RawHeaders', async () => {
    const { response } = await gotoResponse()
    replies[`${FINAL} rawResponseHeaders`] = [
      {
        headers: [
          header('Set-Cookie', 'a=1'),
          header('Set-Cookie', 'b=2'),
          header('X-Dup', '2'),
          header('x-dup', '1'),
        ],
      },
    ]
    expect(await response.allHeaders()).toEqual({
      'set-cookie': 'a=1\nb=2',
      'x-dup': '2, 1',
    })
    expect(await response.headerValue('X-DUP')).toBe('2, 1')
    expect(await response.headerValue('missing')).toBeNull()
    expect(await response.headerValues('set-cookie')).toEqual(['a=1', 'b=2'])
    const array = await response.headersArray()
    array.pop()
    expect((await response.headersArray()).length).toBe(4)
    expect(sent.filter((call) => call.method === 'rawResponseHeaders')).toEqual(
      [{ guid: FINAL, method: 'rawResponseHeaders', params: {}, timeout: 0 }],
    )
  })

  test('tests that body text and json decode the binary as Playwright Buffer does', async () => {
    const { response } = await gotoResponse()
    const bytes = new Uint8Array([
      0x7b, 0x22, 0x68, 0xc3, 0xa9, 0x22, 0x3a, 0x31, 0x7d,
    ])
    const binary = Buffer.from(bytes).toString('base64')
    replies[`${FINAL} body`] = [{ binary }, { binary }, { binary }]
    const body = await response.body()
    expect(body).toBeInstanceOf(Uint8Array)
    expect([...body]).toEqual([...bytes])
    expect(body.toString()).toBe('{"hé":1}')
    expect(body.toString('base64')).toBe(binary)
    expect(body.toString('hex')).toBe(Buffer.from(bytes).toString('hex'))
    expect(JSON.stringify(body)).toBe(JSON.stringify(Buffer.from(bytes)))
    expect(await response.text()).toBe('{"hé":1}')
    expect(await response.json()).toEqual({ hé: 1 })
  })

  test('tests that a failed body read rejects with its apiName', async () => {
    const { response } = await gotoResponse()
    replies[`${FINAL} body`] = [
      { error: { error: { name: 'Error', message: 'unavailable' } }, log: [] },
    ]
    const error = await response.text().catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'Error',
      'response.text: unavailable',
    ])
  })

  test('tests that address, security details and http version follow value || null', async () => {
    const { response } = await gotoResponse()
    replies[`${FINAL} serverAddr`] = [
      { value: { ipAddress: '127.0.0.1', port: 80 } },
      {},
    ]
    replies[`${FINAL} securityDetails`] = [{ value: {} }, {}]
    replies[`${FINAL} httpVersion`] = [{ value: 'HTTP/1.1' }]
    expect(await response.serverAddr()).toEqual({
      ipAddress: '127.0.0.1',
      port: 80,
    })
    expect(await response.serverAddr()).toBeNull()
    expect(await response.securityDetails()).toEqual({})
    expect(await response.securityDetails()).toBeNull()
    expect(await response.httpVersion()).toBe('HTTP/1.1')
  })

  test('tests that finished waits for the request to finish and rejects once the page closed', async () => {
    const { response } = await gotoResponse()
    getterAnswers[`${FINAL} finished`] = ['pending', 'pending', 'finished']
    expect(await response.finished()).toBeNull()
    expect(getters.filter((getter) => getter.field === 'finished').length).toBe(
      3,
    )
    getterAnswers[`${FINAL} finished`] = ['pending', 'closed']
    const error = await response.finished().catch((e) => e)
    expect([error.name, /closed/.test(error.message)]).toEqual(['Error', true])
  })
})

describe('Request', () => {
  test('tests that response() returns the same object goto returned', async () => {
    const { response } = await gotoResponse()
    replies['request@T:L:2 response'] = [{ response: { guid: FINAL } }]
    replies['request@T:L:0 response'] = [
      { response: { guid: 'response@T:L:0' } },
      {},
    ]
    const request = response.request()
    expect(await request.response()).toBe(response)
    const first = request.redirectedFrom()?.redirectedFrom()
    expect(await first?.response()).toBe(first?.existingResponse() ?? undefined)
    expect(await first?.response()).toBeNull()
  })

  test('tests that failure and timing read the live client state', async () => {
    const { response } = await gotoResponse()
    const request = response.request()
    const guid = 'request@T:L:2'
    getterAnswers[`${guid} failure`] = [null, '', 'net::ERR_ABORTED']
    expect(request.failure()).toBeNull()
    expect(request.failure()).toBeNull()
    expect(request.failure()).toEqual({ errorText: 'net::ERR_ABORTED' })
    const live = {
      startTime: 1000,
      domainLookupStart: -1,
      domainLookupEnd: -1,
      connectStart: -1,
      secureConnectionStart: -1,
      connectEnd: -1,
      requestStart: 0.1,
      responseStart: 0.5,
      responseEnd: -1,
    }
    getterAnswers[`${guid} timing`] = [live, { ...live, responseEnd: 0.9 }]
    const timing = request.timing()
    expect(timing).toEqual(live)
    expect(request.timing()).toBe(timing)
    expect(timing.responseEnd).toBe(0.9)
  })

  test('tests that sizes come from the response and fail without one', async () => {
    const { response } = await gotoResponse()
    const sizes = {
      requestBodySize: 0,
      requestHeadersSize: 600,
      responseBodySize: 30,
      responseHeadersSize: 200,
    }
    replies['request@T:L:2 response'] = [{ response: { guid: FINAL } }, {}]
    replies[`${FINAL} sizes`] = [{ sizes }]
    expect(await response.request().sizes()).toEqual(sizes)
    const error = await response
      .request()
      .sizes()
      .catch((e) => e)
    expect(error.name).toBe('Error')
  })

  test('tests that raw request headers answer allHeaders, headersArray and headerValue', async () => {
    const { response } = await gotoResponse()
    replies['request@T:L:2 rawRequestHeaders'] = [
      { headers: [header('Host', 'a.test'), header('Cookie', 'a=1')] },
    ]
    const request = response.request()
    expect(await request.allHeaders()).toEqual({
      host: 'a.test',
      cookie: 'a=1',
    })
    expect(await request.headerValue('HOST')).toBe('a.test')
    expect(await request.headersArray()).toEqual([
      header('Host', 'a.test'),
      header('Cookie', 'a=1'),
    ])
    expect(
      sent.filter((call) => call.method === 'rawRequestHeaders').length,
    ).toBe(1)
  })
})

describe('Buffer', () => {
  /** Every one- and two-byte sequence, plus invalid and truncated multibyte forms. */
  const samples = [
    ...Array.from({ length: 256 }, (_, byte) => [byte]),
    ...Array.from({ length: 256 }, (_, byte) => [0xc3, byte]),
    [0xe2, 0x82, 0xac],
    [0xe2, 0x82],
    [0xf0, 0x9f, 0x98, 0x80],
    [0xf0, 0x9f, 0x98],
    [0xed, 0xa0, 0x80],
    [0xf4, 0x90, 0x80, 0x80],
    [0xe0, 0x80, 0x80],
    [0xc0, 0xaf],
    [0xff, 0xfe, 0x41, 0x00, 0xc3],
  ]

  test('tests that utf8 decoding replaces invalid sequences as Node Buffer does', () => {
    for (const bytes of samples)
      expect(decodeUtf8(new Uint8Array(bytes))).toBe(
        Buffer.from(bytes).toString('utf8'),
      )
  })

  test('tests that toString takes encodings and ranges as Node Buffer does', () => {
    const bytes = [0x68, 0xc3, 0xa9, 0x6c, 0x6c, 0x6f, 0xff]
    const ours = BodyBuffer.fromBinary(Buffer.from(bytes).toString('base64'))
    const bun = Buffer.from(bytes)
    const settle = (read: () => string) => {
      try {
        return read()
      } catch (error) {
        return `throws ${(error as Error).name}`
      }
    }
    const ranges = [
      [],
      [2],
      [-2],
      [1.7, 3.2],
      [0, -1],
      [99],
      [Number.NaN],
      [0, Number.NaN],
      [3, 2],
      [1, 99],
    ]
    const empty = (range: number[]) =>
      [[0, -1], [99], [0, Number.NaN], [3, 2]].some(
        (one) => JSON.stringify(one) === JSON.stringify(range),
      )
    // Bun's Buffer answers every known encoding as Node's does.
    for (const encoding of [
      undefined,
      'UTF-8',
      'Utf8',
      'LATIN1',
      'binary',
      'ascii',
      'Base64',
      'base64url',
      'HEX',
      'ucs2',
      'UCS-2',
      'utf16le',
      'utf-16le',
    ])
      for (const range of ranges) {
        const args = [encoding, ...range] as never[]
        expect(
          settle(() => ours.toString(...args)),
          JSON.stringify(args),
        ).toBe(settle(() => bun.toString(...args)))
      }
    // An unknown encoding is a TypeError, but Node reads the range first (lib/buffer.js), so an
    // empty range is '' whatever the encoding; Bun's Buffer throws there. These are Node 22's.
    for (const encoding of ['nope', null])
      for (const range of ranges) {
        const args = [encoding, ...range] as never[]
        expect(
          settle(() => ours.toString(...args)),
          JSON.stringify(args),
        ).toBe(empty(range) ? '' : 'throws TypeError')
      }
    expect(ours.toString()).toBe(bun.toString())
  })

  test('tests that toString encodings match Node Buffer', () => {
    const bytes = [0x00, 0x41, 0x7f, 0x80, 0xc3, 0xa9, 0xff]
    const ours = BodyBuffer.fromBinary(Buffer.from(bytes).toString('base64'))
    for (const encoding of [
      'utf8',
      'utf-8',
      'latin1',
      'binary',
      'ascii',
      'base64',
      'hex',
    ] as const)
      expect(ours.toString(encoding)).toBe(
        Buffer.from(bytes).toString(encoding),
      )
    expect(ours.toString('utf8', 1, 3)).toBe('A\u007f')
    expect(ours.equals(new Uint8Array(bytes))).toBe(true)
    expect(ours.slice(1, 2).toString()).toBe('A')
    expect(() => ours.toString('nope' as never)).toThrow(
      'Unknown encoding: nope',
    )
  })
})
