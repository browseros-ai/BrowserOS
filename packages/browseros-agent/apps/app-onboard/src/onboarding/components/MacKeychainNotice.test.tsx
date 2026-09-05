import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import {
  MacKeychainPermissionNote,
  MacKeychainPreview,
  MacKeychainReminder,
} from './MacKeychainNotice'

describe('MacKeychainPreview', () => {
  it('illustrates the native prompt without collecting credentials or adding controls', () => {
    const html = renderToStaticMarkup(<MacKeychainPreview />)
    expect(html).toContain('Example macOS dialog')
    expect(html).toContain('Chrome Safe Storage')
    expect(html).toContain('Always Allow')
    expect(html.indexOf('Always Allow')).toBeLessThan(html.indexOf('Deny'))
    expect(html).not.toMatch(/<(img|input|button)\b/)
    expect(html).not.toContain('tabindex')
  })

  it('explains that Allow applies to this import', () => {
    expect(renderToStaticMarkup(<MacKeychainPermissionNote />)).toContain(
      'Allow gives access for this import.',
    )
  })

  it('keeps the progress reminder conditional because prompt state is unknown', () => {
    const html = renderToStaticMarkup(<MacKeychainReminder />)
    expect(html).toContain('If macOS asks')
    expect(html).toContain('Mac login password')
    expect(html).toContain('Allow')
    expect(html).not.toContain('Always Allow')
  })
})
