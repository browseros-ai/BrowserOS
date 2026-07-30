import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MacKeychainNotice } from './MacKeychainNotice'

describe('MacKeychainNotice', () => {
  it('renders the keychain screenshot from the chromium-allowlisted icon path', () => {
    const html = renderToStaticMarkup(<MacKeychainNotice />)

    // A literal /icon/... path keeps the chromium build allowlist happy; an
    // import would emit a new hashed asset and fail verify-chromium-build.ts.
    expect(html).toContain('src="/icon/keychain-prompt.png"')
  })

  // The importer reads the Keychain once per run, so Always Allow is both
  // unnecessary and a permanent ACL grant for a one-off step. The screenshot
  // highlights Allow, and copy that named the other button would contradict
  // the figure sitting right beneath it.
  it('tells the user to click Allow, not Always Allow', () => {
    const html = renderToStaticMarkup(<MacKeychainNotice />)

    expect(html).toContain('Allow')
    expect(html).not.toContain('Always Allow')
    expect(html).not.toContain('only asks once')
  })

  it('warns that a password is required', () => {
    const html = renderToStaticMarkup(<MacKeychainNotice />)

    expect(html).toContain('macOS will ask for your password')
  })
})
