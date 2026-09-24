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
})
