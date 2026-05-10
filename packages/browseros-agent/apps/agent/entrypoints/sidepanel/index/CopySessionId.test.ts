/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * TDD tests for Copy Session ID feature.
 */

import { afterEach, describe, expect, mock, test } from 'bun:test'
import {
  buildSessionIdLabel,
  copySessionIdToClipboard,
  SESSION_ID_COPIED_EVENT,
} from './CopySessionId'

// ---------------------------------------------------------------------------
// copySessionIdToClipboard
// ---------------------------------------------------------------------------
describe('copySessionIdToClipboard', () => {
  afterEach(() => {
    mock.restore()
  })

  test('copies a valid UUID conversationId to the clipboard', async () => {
    //#given
    const conversationId = '550e8400-e29b-41d4-a716-446655440000'
    const writeText = mock(() => Promise.resolve(undefined))
    // @ts-expect-error stubbing for test
    globalThis.navigator = { clipboard: { writeText } }

    //#when
    await copySessionIdToClipboard(conversationId)

    //#then
    expect(writeText).toHaveBeenCalled()
    expect(writeText.mock.calls[0][0]).toBe(conversationId)
  })

  test('returns false for empty string conversationId and does not copy', async () => {
    //#given
    const writeText = mock(() => Promise.resolve(undefined))
    // @ts-expect-error stubbing for test
    globalThis.navigator = { clipboard: { writeText } }

    //#when
    const result = await copySessionIdToClipboard('')

    //#then
    expect(writeText).not.toHaveBeenCalled()
    expect(result).toBe(false)
  })

  test('returns false for null conversationId and does not copy', async () => {
    //#given
    const writeText = mock(() => Promise.resolve(undefined))
    // @ts-expect-error stubbing for test
    globalThis.navigator = { clipboard: { writeText } }

    //#when
    // @ts-expect-error — intentionally passing null
    const result = await copySessionIdToClipboard(null)

    //#then
    expect(writeText).not.toHaveBeenCalled()
    expect(result).toBe(false)
  })

  test('handles very long IDs (> 1000 chars) by still copying them', async () => {
    //#given
    const longId = 'a'.repeat(2048)
    const writeText = mock(() => Promise.resolve(undefined))
    // @ts-expect-error stubbing for test
    globalThis.navigator = { clipboard: { writeText } }

    //#when
    await copySessionIdToClipboard(longId)

    //#then
    expect(writeText).toHaveBeenCalledWith(longId)
  })
})

// ---------------------------------------------------------------------------
// buildSessionIdLabel
// ---------------------------------------------------------------------------
describe('buildSessionIdLabel', () => {
  test('truncates a UUID to first 4 and last 4 chars with ellipsis', () => {
    //#given
    const id = '550e8400-e29b-41d4-a716-446655440000'

    //#when
    const label = buildSessionIdLabel(id)

    //#then
    expect(label).toBe('550e...0000')
  })

  test('returns full id when it is shorter than maxLength', () => {
    //#given
    const shortId = 'abc'

    //#when
    const label = buildSessionIdLabel(shortId)

    //#then
    expect(label).toBe('abc')
  })

  test('respects custom maxLength to extract head and tail segments', () => {
    //#given
    const id = '550e8400-e29b-41d4-a716-446655440000'

    //#when
    const label = buildSessionIdLabel(id, 6)

    //#then
    // headLen = 3, tailLen = 3 → "550...000"
    expect(label).toContain('...')
    expect(label.length).toBe(3 + 3 + 3) // head + ... + tail
  })

  test('returns empty string for empty input', () => {
    //#given — empty string

    //#when
    const label = buildSessionIdLabel('')

    //#then
    expect(label).toBe('')
  })

  test('returns "—" for null/undefined input', () => {
    //#given — nullish input

    //#when
    // @ts-expect-error — intentionally passing null
    const labelNull = buildSessionIdLabel(null)
    // @ts-expect-error — intentionally passing undefined
    const labelUndef = buildSessionIdLabel(undefined)

    //#then
    expect(labelNull).toBe('—')
    expect(labelUndef).toBe('—')
  })
})

// ---------------------------------------------------------------------------
// SESSION_ID_COPIED_EVENT constant
// ---------------------------------------------------------------------------
describe('SESSION_ID_COPIED_EVENT', () => {
  test('exports the analytics event constant', () => {
    //#given
    const expectedEventName = 'sidepanel.session_id.copied'

    //#when
    const eventName = SESSION_ID_COPIED_EVENT

    //#then
    expect(eventName).toBe(expectedEventName)
  })
})
