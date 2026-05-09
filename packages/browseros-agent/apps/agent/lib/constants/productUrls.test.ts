import { describe, expect, it } from 'bun:test'
import {
  cloudSyncHelpUrl,
  cloudSyncSignInLinks,
  privacyPolicyUrl,
  termsOfServiceUrl,
} from './productUrls'

describe('cloud sync sign-in links', () => {
  it('points to the public legal and cloud sync documentation URLs', () => {
    expect(termsOfServiceUrl).toBe('https://browseros.com/terms')
    expect(privacyPolicyUrl).toBe('https://browseros.com/privacy')
    expect(cloudSyncHelpUrl).toBe(
      'https://docs.browseros.com/features/sync-to-cloud',
    )
  })

  it('includes legal and cloud sync documentation links in display order', () => {
    expect(cloudSyncSignInLinks).toEqual([
      { label: 'Terms of Service', url: termsOfServiceUrl },
      { label: 'Privacy Policy', url: privacyPolicyUrl },
      { label: 'Learn more about cloud sync', url: cloudSyncHelpUrl },
    ])
  })
})
