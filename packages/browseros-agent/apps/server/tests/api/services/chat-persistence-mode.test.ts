import { describe, expect, test } from 'bun:test'
import { shouldPersist } from '../../../src/api/services/chat-persistence-mode'

describe('shouldPersist', () => {
  test('persists when the request says so', () => {
    expect(shouldPersist({ persist: true })).toBe(true)
  })

  test('stays stateless for incognito and temporary chats', () => {
    expect(shouldPersist({ persist: false })).toBe(false)
  })

  // The field this replaced. The extension updates independently of the
  // browser binary, so a shipped build may still send it.
  test('still reads the shape it replaced', () => {
    expect(shouldPersist({ historyMode: 'local' })).toBe(true)
    expect(shouldPersist({ historyMode: 'cloud' })).toBe(false)
  })

  // historyMode defaulted to 'cloud', so omitting it meant no history at all.
  // That was safe only while a cloud still held the conversation.
  test('persists when the request says nothing', () => {
    expect(shouldPersist({})).toBe(true)
  })

  test('prefers the current field when a client sends both', () => {
    expect(shouldPersist({ persist: false, historyMode: 'local' })).toBe(false)
    expect(shouldPersist({ persist: true, historyMode: 'cloud' })).toBe(true)
  })

  test('keeps a scheduled run out of the conversation list', () => {
    // The scheduled caller sends neither field, so before the default flipped
    // it fell through to not persisting. Its record is the run history.
    expect(shouldPersist({ isScheduledTask: true })).toBe(false)
  })

  test('keeps a scheduled run out even when the caller asks to persist', () => {
    // Nothing sends this combination today. It resolves the same way if
    // something ever does, rather than by whichever check is written first.
    expect(shouldPersist({ isScheduledTask: true, persist: true })).toBe(false)
    expect(shouldPersist({ isScheduledTask: true, historyMode: 'local' })).toBe(
      false,
    )
  })

  test('persists an ordinary chat that omits everything', () => {
    expect(shouldPersist({ isScheduledTask: false })).toBe(true)
  })
})
