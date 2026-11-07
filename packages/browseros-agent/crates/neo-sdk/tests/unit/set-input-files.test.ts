/** setInputFiles and drop: Playwright's client conversion, wire params and public names. */
import { expect, test } from 'bun:test'
import { BrowserContext } from '../../src/browser-context'
import { Connection } from '../../src/connection'
import { EventPipe } from '../../src/events'
import { Page } from '../../src/page'
import * as protocol from '../../src/protocol'
import {
  convertDropPayload,
  convertInputFiles,
} from '../../src/set-input-files'

const bytes = (text: string) =>
  Uint8Array.from(text, (character) => character.charCodeAt(0))

function start() {
  const connection = new Connection(0)
  const sent: { api: string; method: string; params: unknown }[] = []
  connection.send = (async (
    api: string,
    _guid: string,
    method: string,
    params: unknown,
  ) => {
    sent.push({ api, method, params })
    return {}
  }) as never
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext('browser-context@T', connection, new EventPipe()),
    connection,
  )
  return { page, sent }
}

test('tests that files convert as Playwright converts them', () => {
  expect(convertInputFiles('a.txt')).toEqual({ localPaths: ['a.txt'] })
  expect(convertInputFiles(['a.txt', 'dir'])).toEqual({
    localPaths: ['a.txt', 'dir'],
  })
  expect(convertInputFiles([])).toEqual({ payloads: [] })
  expect(
    convertInputFiles([
      { name: 'a.txt', mimeType: 'text/plain', buffer: bytes('hi') },
      { name: 'b.bin', buffer: bytes('') } as never,
    ]),
  ).toEqual({
    payloads: [
      { name: 'a.txt', mimeType: 'text/plain', buffer: 'aGk=' },
      { name: 'b.bin', buffer: '' },
    ],
  })
  expect(() =>
    convertInputFiles([
      'a.txt',
      { name: 'b', mimeType: 'text/plain', buffer: bytes('b') },
    ] as never),
  ).toThrow('File paths cannot be mixed with buffers')
  // Playwright's size check reads `byteLength` only, before any validation.
  expect(() =>
    convertInputFiles([
      { name: 'big', mimeType: 'x', buffer: { byteLength: 50 * 1024 * 1024 } },
    ] as never),
  ).toThrow(
    'Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.',
  )
  // A null item fails Playwright's size check first, reading its `buffer`: a TypeError.
  expect(() => convertInputFiles(null as never)).toThrow(TypeError)
  for (const [payload, message] of [
    [7, 'payloads[0]: expected object, got number'],
    [
      { name: 1, buffer: bytes('') },
      'payloads[0].name: expected string, got number',
    ],
    [
      { name: 'a', mimeType: 2, buffer: bytes('') },
      'payloads[0].mimeType: expected string, got number',
    ],
    [
      { name: 'a', buffer: 'text' },
      'payloads[0].buffer: expected Buffer, got string',
    ],
  ] as const) {
    expect(() => convertInputFiles(payload as never)).toThrow(message)
  }
})

test('tests that drop payloads convert files and data as Playwright does', () => {
  expect(
    convertDropPayload({ data: { 'text/plain': 'x', 'text/html': '<b>' } }),
  ).toEqual({
    data: [
      { mimeType: 'text/plain', value: 'x' },
      { mimeType: 'text/html', value: '<b>' },
    ],
  })
  expect(convertDropPayload({ files: 'a.txt' })).toEqual({
    payloads: undefined,
    localPaths: ['a.txt'],
    data: undefined,
  })
  expect(() =>
    convertDropPayload({ data: { 'text/plain': 1 } } as never),
  ).toThrow('data[0].value: expected string, got number')
})

test('tests that setInputFiles and drop send Playwright params from every receiver', async () => {
  const { page, sent } = start()
  const file = { name: 'a.txt', mimeType: 'text/plain', buffer: bytes('hi') }
  await page.setInputFiles('#f', file, { timeout: 5 })
  await page.mainFrame().setInputFiles('#f', 'up/a.txt')
  await page.locator('#f').setInputFiles([])
  await page
    .locator('#zone')
    .drop(
      { files: file, data: { 'text/plain': 'x' } },
      { position: { x: 1, y: 2 } },
    )
  expect(sent).toEqual([
    {
      api: 'page.setInputFiles',
      method: 'setInputFiles',
      params: {
        selector: '#f',
        payloads: [{ name: 'a.txt', mimeType: 'text/plain', buffer: 'aGk=' }],
      },
    },
    {
      api: 'frame.setInputFiles',
      method: 'setInputFiles',
      params: { selector: '#f', localPaths: ['up/a.txt'] },
    },
    {
      api: 'locator.setInputFiles',
      method: 'setInputFiles',
      params: { selector: '#f', strict: true, payloads: [] },
    },
    {
      api: 'locator.drop',
      method: 'drop',
      params: {
        selector: '#zone',
        strict: true,
        position: { x: 1, y: 2 },
        payloads: [{ name: 'a.txt', mimeType: 'text/plain', buffer: 'aGk=' }],
        data: [{ mimeType: 'text/plain', value: 'x' }],
      },
    },
  ])
})

test('tests that the file key lists follow the Playwright schema', () => {
  expect(protocol.FRAME_SET_INPUT_FILES_KEYS).toEqual([
    'selector',
    'strict',
    'payloads',
    'localDirectory',
    'directoryStream',
    'localPaths',
    'streams',
  ])
  expect(protocol.FRAME_DROP_KEYS).toEqual([
    'selector',
    'strict',
    'position',
    'payloads',
    'localPaths',
    'streams',
    'data',
  ])
})
