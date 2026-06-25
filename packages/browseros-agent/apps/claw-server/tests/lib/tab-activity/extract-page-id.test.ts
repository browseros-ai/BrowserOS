import { describe, expect, it } from 'bun:test'
import { extractPageId } from '../../../src/lib/tab-activity/extract-page-id'

describe('extractPageId', () => {
  it('returns the page id for every tool that takes a page arg', () => {
    for (const tool of [
      'act',
      'diff',
      'download',
      'evaluate',
      'grep',
      'navigate',
      'pdf',
      'read',
      'screenshot',
      'snapshot',
      'tabs',
      'upload',
      'wait',
    ]) {
      expect(extractPageId(tool, { page: 7 })).toBe(7)
    }
  })

  it('returns null for tools without a page arg', () => {
    expect(extractPageId('tab_groups', { page: 7 })).toBeNull()
    expect(extractPageId('windows', { page: 7 })).toBeNull()
    expect(extractPageId('run', { page: 7 })).toBeNull()
  })

  it('returns null for unknown tools', () => {
    expect(extractPageId('completely_unknown', { page: 1 })).toBeNull()
  })

  it('returns null when page is missing', () => {
    expect(extractPageId('navigate', { url: 'https://example.com' })).toBeNull()
    expect(extractPageId('navigate', {})).toBeNull()
  })

  it('returns null when page is not a number', () => {
    expect(extractPageId('navigate', { page: '7' })).toBeNull()
    expect(extractPageId('navigate', { page: null })).toBeNull()
    expect(extractPageId('navigate', { page: undefined })).toBeNull()
  })

  it('returns null for non-integer page', () => {
    expect(extractPageId('navigate', { page: 1.5 })).toBeNull()
  })

  it('returns null for non-positive page', () => {
    expect(extractPageId('navigate', { page: 0 })).toBeNull()
    expect(extractPageId('navigate', { page: -1 })).toBeNull()
  })

  it('returns null for non-object args', () => {
    expect(extractPageId('navigate', null)).toBeNull()
    expect(extractPageId('navigate', undefined)).toBeNull()
    expect(extractPageId('navigate', 'page=1')).toBeNull()
    expect(extractPageId('navigate', 42)).toBeNull()
  })
})
