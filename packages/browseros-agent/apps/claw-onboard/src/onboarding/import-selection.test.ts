import { describe, expect, it } from 'bun:test'
import type { BrowserOSImportSource } from './browseros-onboarding-api'
import {
  browserShortName,
  defaultImportItemsForSource,
  importChipsFor,
  importDoneLine,
  importItemLabel,
  needsMacKeychainPermission,
  OPTIONAL_IMPORT_ITEMS,
  offeredSource,
  sanitizeImportSelection,
  startImportRequestFor,
} from './import-selection'
import { MOCK_BROWSEROS_IMPORT_SOURCES } from './mock-import-sources'

const [WORK, PERSONAL, EDGE] = MOCK_BROWSEROS_IMPORT_SOURCES

describe('mock import sources', () => {
  it('ship stable ids in the Chromium contract shape', () => {
    expect(MOCK_BROWSEROS_IMPORT_SOURCES.map((source) => source.id)).toEqual([
      'chrome-work',
      'chrome-personal',
      'edge-default',
    ])
    for (const source of MOCK_BROWSEROS_IMPORT_SOURCES) {
      expect(source.displayName.length).toBeGreaterThan(0)
      expect(source.supportedItems).toContain(source.recommendedItems[0])
    }
  })
})

describe('importChipsFor', () => {
  it('orders logins first and marks the opt-ins', () => {
    expect(
      importChipsFor(WORK).map((chip) => [chip.label, chip.optional]),
    ).toEqual([
      ['Signed-in sites', false],
      ['Passwords', false],
      ['Bookmarks', false],
      ['History', false],
      ['Autofill', false],
      ['Extensions', true],
      ['Search engines', true],
    ])
  })

  it('shows only what the source supports', () => {
    expect(importChipsFor(EDGE).map((chip) => chip.item)).toEqual([
      'cookies',
      'passwords',
      'bookmarks',
      'history',
    ])
  })

  it('keeps an unknown native item readable instead of dropping it', () => {
    const newer = {
      ...WORK,
      supportedItems: ['history', 'savedWindows'],
    } as unknown as BrowserOSImportSource
    expect(importChipsFor(newer).map((chip) => chip.label)).toEqual([
      'History',
      'Saved windows',
    ])
    expect(importItemLabel('')).toBe('Other data')
  })
})

describe('default selection', () => {
  it('treats only search engines and extensions as opt-ins', () => {
    expect([...OPTIONAL_IMPORT_ITEMS]).toEqual(['searchEngines', 'extensions'])
  })

  it('turns on every supported item except the opt-ins', () => {
    expect(defaultImportItemsForSource(WORK)).toEqual([
      'history',
      'bookmarks',
      'cookies',
      'passwords',
      'autofill',
    ])
  })

  // Native sends recommendedItems identical to supportedItems, so following
  // it would switch the opt-ins on.
  it('ignores recommendedItems that would switch the opt-ins on', () => {
    expect(
      defaultImportItemsForSource({
        ...WORK,
        supportedItems: ['extensions', 'history'],
        recommendedItems: ['extensions', 'history'],
      }),
    ).toEqual(['history'])
  })

  it('sanitizes selections to supported items in source order', () => {
    expect(
      sanitizeImportSelection(PERSONAL, ['autofill', 'history', 'extensions']),
    ).toEqual(['history', 'autofill'])
  })

  it('builds no import request for an empty selection', () => {
    expect(startImportRequestFor(WORK, [])).toBeNull()
    expect(startImportRequestFor(WORK, ['history', 'history'])).toEqual({
      sourceId: 'chrome-work',
      items: ['history'],
    })
  })
})

describe('copy helpers', () => {
  it('shortens Google Chrome and keeps other browsers whole', () => {
    expect(browserShortName(WORK)).toBe('Chrome')
    expect(browserShortName(EDGE)).toBe('Microsoft Edge')
  })

  it('writes the done line from completed items only', () => {
    expect(importDoneLine(['history', 'cookies'])).toBe(
      'Done. Your logins are here.',
    )
    expect(importDoneLine(['bookmarks', 'history'])).toBe(
      'Done. Your bookmarks and history are here.',
    )
    // Round 4's line whenever bookmarks and history both came over.
    expect(importDoneLine(['bookmarks', 'history', 'autofill'])).toBe(
      'Done. Your bookmarks and history are here.',
    )
    expect(importDoneLine(['bookmarks', 'autofill'])).toBe(
      'Done. Your bookmarks and autofill are here.',
    )
    expect(importDoneLine(['autofill'])).toBe('Done. Your autofill is here.')
    expect(importDoneLine(['bookmarks'])).toBe('Done. Your bookmarks are here.')
    expect(importDoneLine([])).toBe('Done.')
  })
})

describe('offeredSource', () => {
  it('drops logins on Windows only', () => {
    expect(offeredSource(WORK, 'windows').supportedItems).toEqual([
      'history',
      'bookmarks',
      'searchEngines',
      'autofill',
      'extensions',
    ])
    expect(offeredSource(WORK, 'windows').recommendedItems).not.toContain(
      'passwords',
    )
    expect(offeredSource(WORK, 'mac')).toBe(WORK)
    expect(offeredSource(WORK, 'other')).toBe(WORK)
  })
})

describe('needsMacKeychainPermission', () => {
  it('gates only macOS Chrome imports containing supported encrypted data', () => {
    expect(needsMacKeychainPermission(true, WORK, ['cookies'])).toBe(true)
    expect(needsMacKeychainPermission(true, WORK, ['passwords'])).toBe(true)
    for (const browserName of ['Safari', 'Microsoft Edge', 'Chromium']) {
      expect(
        needsMacKeychainPermission(true, { ...WORK, browserName }, ['cookies']),
      ).toBe(false)
    }
    expect(needsMacKeychainPermission(false, WORK, ['cookies'])).toBe(false)
    expect(needsMacKeychainPermission(true, undefined, ['cookies'])).toBe(false)
    expect(needsMacKeychainPermission(true, WORK, ['history'])).toBe(false)
    expect(
      needsMacKeychainPermission(
        true,
        { ...WORK, supportedItems: ['history'] },
        ['cookies'],
      ),
    ).toBe(false)
  })
})
