import { describe, expect, it } from 'bun:test'
import {
  conversationTitle,
  historyDateGroup,
  historyList,
  historyTimestamp,
} from './history-list'

const now = new Date(2026, 8, 8, 12)
const row = (id: string, date: Date, text = id) => ({
  id,
  lastMessagedAt: date.getTime(),
  lastUserMessage: text,
})

describe('local history navigation', () => {
  it('sorts recent first, caps the combined groups, and searches older rows before capping', () => {
    const conversations = [
      row('old', new Date(2026, 8, 1), 'Weekend trip'),
      row('yesterday', new Date(2026, 8, 7, 20)),
      row('today', new Date(2026, 8, 8, 11)),
    ]
    expect(historyList(conversations, '', 2, now)).toEqual({
      groups: [
        { label: 'Today', conversations: [conversations[2]] },
        { label: 'Yesterday', conversations: [conversations[1]] },
      ],
      hasMore: true,
      total: 3,
    })
    const result = historyList(conversations, ' WEEKEND ', 2, now)
    expect(result.groups[0]).toEqual({
      label: 'Earlier',
      conversations: [conversations[0]],
    })
    expect(result.hasMore).toBe(false)
    expect(conversations[0]?.id).toBe('old')
  })

  it('groups a message before midnight as yesterday, even a minute later', () => {
    const midnight = new Date(2026, 8, 8, 0, 0)
    expect(
      historyDateGroup(new Date(2026, 8, 7, 23, 59).getTime(), midnight),
    ).toBe('Yesterday')
    expect(
      historyDateGroup(new Date(2026, 8, 6, 23, 59).getTime(), midnight),
    ).toBe('Earlier')
  })

  it('uses the previous calendar day across DST and year boundaries', () => {
    const january = new Date(2027, 0, 1, 1)
    expect(historyDateGroup(new Date(2026, 11, 31, 2).getTime(), january)).toBe(
      'Yesterday',
    )
    const afterSpringChange = new Date(2026, 2, 9, 0, 10)
    expect(
      historyDateGroup(new Date(2026, 2, 8, 0, 1).getTime(), afterSpringChange),
    ).toBe('Yesterday')
  })

  it('moves a continued conversation into Today without duplicating its id', () => {
    const before = [
      row('trip', new Date(2026, 8, 7)),
      row('email', new Date(2026, 8, 8, 9)),
    ]
    const after = before.map((item) =>
      item.id === 'trip' ? row('trip', now, 'Make it a three-day trip') : item,
    )
    const result = historyList(after, '', 6, now)
    expect(result.groups).toHaveLength(1)
    expect(result.groups[0]?.conversations.map((item) => item.id)).toEqual([
      'trip',
      'email',
    ])
    expect(result.groups[0]?.conversations[0]?.lastUserMessage).toBe(
      'Make it a three-day trip',
    )
  })

  it('handles untitled, empty and unmatched histories', () => {
    expect(conversationTitle('  ')).toBe('Untitled conversation')
    expect(historyList([], '', 6, now)).toEqual({
      groups: [],
      total: 0,
      hasMore: false,
    })
    expect(historyList([row('1', now)], 'missing', 6, now).total).toBe(0)
    expect(historyList([row('1', now, '')], 'untitled', 6, now).total).toBe(1)
  })

  it('formats recent timestamps without negative ages', () => {
    expect(historyTimestamp(now.getTime() + 1000, now)).toBe('Just now')
    expect(historyTimestamp(now.getTime() - 60_000, now)).toBe('1 minute ago')
    expect(historyTimestamp(now.getTime() - 120_000, now)).toBe('2 minutes ago')
    expect(historyTimestamp(now.getTime() - 3_600_000, now)).toBe('1 hour ago')
  })
})
