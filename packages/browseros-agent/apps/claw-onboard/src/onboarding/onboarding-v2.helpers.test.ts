import { describe, expect, it } from 'bun:test'
import {
  completedImportItemCount,
  DEFAULT_BROWSEROS_IMPORT_SOURCE_ID,
  importItemLabel,
  importItemListLabel,
  importProgressTotal,
  importSourceSelectionChangeFor,
  MOCK_BROWSEROS_IMPORT_SOURCES,
  STARTER_PROMPTS,
  sanitizeImportSelection,
  selectableItemsForSource,
  selectedSourceById,
  startImportRequestFor,
} from './onboarding-v2.helpers'

describe('MOCK_BROWSEROS_IMPORT_SOURCES fixture', () => {
  it('ships mock import sources with stable ids', () => {
    expect(MOCK_BROWSEROS_IMPORT_SOURCES.map((source) => source.id)).toEqual([
      'chrome-work',
      'chrome-personal',
      'edge-default',
    ])
    expect(DEFAULT_BROWSEROS_IMPORT_SOURCE_ID).toBe('chrome-work')
  })

  it('uses the Chromium contract source shape', () => {
    for (const source of MOCK_BROWSEROS_IMPORT_SOURCES) {
      expect(source.displayName.length).toBeGreaterThan(0)
      expect(source.recommendedItems.length).toBeGreaterThan(0)
      expect(source.supportedItems).toContain(source.recommendedItems[0])
    }
  })
})

describe('source selection helpers', () => {
  it('finds the selected source by contract id', () => {
    expect(
      selectedSourceById(MOCK_BROWSEROS_IMPORT_SOURCES, 'chrome-personal')
        ?.displayName,
    ).toBe('Google Chrome - Personal')
  })

  it('clears the selected source and items when sources empty', () => {
    expect(importSourceSelectionChangeFor([], 'chrome-work')).toEqual({
      selectedSourceId: '',
      selectedItems: [],
    })
  })

  it('selects the first source with default items when the current source vanished', () => {
    expect(
      importSourceSelectionChangeFor(
        MOCK_BROWSEROS_IMPORT_SOURCES,
        'missing-source',
      ),
    ).toEqual({
      selectedSourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
      selectedItems: selectableItemsForSource(MOCK_BROWSEROS_IMPORT_SOURCES[0]),
    })
  })

  it('does not change selection when the current source still exists', () => {
    expect(
      importSourceSelectionChangeFor(
        MOCK_BROWSEROS_IMPORT_SOURCES,
        'chrome-personal',
      ),
    ).toBeNull()
  })

  it('falls back to supported items when recommended items are empty', () => {
    expect(
      selectableItemsForSource({
        ...MOCK_BROWSEROS_IMPORT_SOURCES[0],
        recommendedItems: [],
      }),
    ).toEqual(MOCK_BROWSEROS_IMPORT_SOURCES[0].supportedItems)
  })

  it('sanitizes explicit item selections against supported items in source order', () => {
    expect(
      sanitizeImportSelection(MOCK_BROWSEROS_IMPORT_SOURCES[0], [
        'extensions',
        'history',
        'bookmarks',
        'history',
        'autofill',
      ]),
    ).toEqual(['history', 'bookmarks', 'autofill', 'extensions'])
  })

  it('filters unsupported selected items and returns empty for no matches', () => {
    expect(
      sanitizeImportSelection(MOCK_BROWSEROS_IMPORT_SOURCES[2], [
        'extensions',
        'autofill',
        'history',
      ]),
    ).toEqual(['history'])
    expect(
      sanitizeImportSelection(MOCK_BROWSEROS_IMPORT_SOURCES[2], [
        'extensions',
        'autofill',
      ]),
    ).toEqual([])
  })

  it('builds the Chromium start-import request for one source', () => {
    expect(startImportRequestFor(MOCK_BROWSEROS_IMPORT_SOURCES[0])).toEqual({
      sourceId: 'chrome-work',
      items: MOCK_BROWSEROS_IMPORT_SOURCES[0].recommendedItems,
    })
  })

  it('builds the Chromium start-import request for a sanitized subset', () => {
    expect(
      startImportRequestFor(MOCK_BROWSEROS_IMPORT_SOURCES[0], [
        'extensions',
        'history',
      ]),
    ).toEqual({
      sourceId: 'chrome-work',
      items: ['history', 'extensions'],
    })
  })

  it('does not build a start-import request for empty explicit selections', () => {
    expect(
      startImportRequestFor(MOCK_BROWSEROS_IMPORT_SOURCES[0], []),
    ).toBeNull()
    expect(
      startImportRequestFor(MOCK_BROWSEROS_IMPORT_SOURCES[2], ['extensions']),
    ).toBeNull()
  })

  it('does not build a start-import request for empty item sources', () => {
    expect(
      startImportRequestFor({
        ...MOCK_BROWSEROS_IMPORT_SOURCES[0],
        recommendedItems: [],
        supportedItems: [],
      }),
    ).toBeNull()
  })
})

describe('import item display helpers', () => {
  it('formats import item labels for source tiles and summaries', () => {
    expect(importItemListLabel(['history', 'bookmarks', 'cookies'])).toBe(
      'History, Bookmarks, Cookies',
    )
  })

  it('falls back to readable labels for unknown Chromium item strings', () => {
    expect(importItemLabel('savedWindows')).toBe('Saved Windows')
    expect(importItemListLabel(['history', 'savedWindows'])).toBe(
      'History, Saved Windows',
    )
  })

  it('uses Chromium progress totals when present', () => {
    expect(
      importProgressTotal(2, {
        currentItem: 'cookies',
        completedItems: ['history', 'bookmarks'],
        totalItems: 7,
      }),
    ).toBe(7)
    expect(
      completedImportItemCount({
        currentItem: 'cookies',
        completedItems: ['history', 'bookmarks'],
        totalItems: 7,
      }),
    ).toBe(2)
  })

  it('falls back to selected item count when Chromium omits progress totals', () => {
    expect(importProgressTotal(2, undefined)).toBe(2)
  })
})

describe('STARTER_PROMPTS', () => {
  it('ships at least two suggestions for the Ready step', () => {
    expect(STARTER_PROMPTS.length).toBeGreaterThanOrEqual(2)
  })
})
