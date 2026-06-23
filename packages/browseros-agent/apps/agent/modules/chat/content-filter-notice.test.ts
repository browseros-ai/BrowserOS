import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  addContentFilterNotice,
  CONTENT_FILTER_NOTICE,
} from './content-filter-notice'

const userMessage: UIMessage = {
  id: 'user-1',
  role: 'user',
  parts: [{ type: 'text', text: 'hello' }],
}

function assistant(parts: UIMessage['parts']): UIMessage {
  return {
    id: 'assistant-1',
    role: 'assistant',
    parts,
  }
}

describe('addContentFilterNotice', () => {
  it('adds a visible notice for a content-filtered response with no text', () => {
    const response = assistant([{ type: 'step-start' }])
    const messages = [userMessage, response]

    const next = addContentFilterNotice(messages, response, 'content-filter')

    expect(next).not.toBe(messages)
    expect(next[1]?.parts).toEqual([
      { type: 'step-start' },
      { type: 'text', text: CONTENT_FILTER_NOTICE },
    ])
  })

  it('does not change content-filtered responses that already have text', () => {
    const response = assistant([{ type: 'text', text: 'visible answer' }])
    const messages = [userMessage, response]

    expect(addContentFilterNotice(messages, response, 'content-filter')).toBe(
      messages,
    )
  })

  it('does not change non content-filter responses', () => {
    const response = assistant([])
    const messages = [userMessage, response]

    expect(addContentFilterNotice(messages, response, 'stop')).toBe(messages)
  })

  it('appends a notice response when the filtered message is not in the list', () => {
    const response = assistant([])

    const next = addContentFilterNotice(
      [userMessage],
      response,
      'content-filter',
    )

    expect(next).toEqual([
      userMessage,
      {
        ...response,
        parts: [{ type: 'text', text: CONTENT_FILTER_NOTICE }],
      },
    ])
  })
})
