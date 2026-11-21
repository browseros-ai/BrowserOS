import { describe, expect, test } from 'bun:test'
import { join, normalize } from 'node:path'
import {
  parseProfilesIni,
  pickDefaultProfile,
  resolveProfile,
} from '../src/firefox/profiles'
import { cdpPortFromLocalState } from '../src/neo/discover'

const ROOT = normalize('/ff')

// Shape of a real multi-install profiles.ini: the legacy Default=1 points at
// an old profile while each install names its own default.
const INI = `[General]
StartWithLastProfile=1
Version=2

[Install308046B0AF4A39CB]
Default=Profiles/q6flhfhs.default-release
Locked=1

[Profile1]
Name=default
IsRelative=1
Path=Profiles/6vaq0wfi.default
Default=1

[Profile0]
Name=default-release
IsRelative=1
Path=Profiles/q6flhfhs.default-release

[InstallCA9422711AE1A81C]
Default=Profiles/khig5ure.dev-edition-default

[Profile2]
Name=dev-edition-default
IsRelative=1
Path=Profiles/khig5ure.dev-edition-default

[Profile3]
Name=elsewhere
IsRelative=0
Path=${normalize('/other/place')}
`

describe('profiles.ini', () => {
  const profiles = parseProfilesIni(INI, ROOT)

  test('parses every profile with install and legacy defaults', () => {
    expect(
      profiles.map((p) => [p.name, p.installDefault, p.legacyDefault]),
    ).toEqual([
      ['default', false, true],
      ['default-release', true, false],
      ['dev-edition-default', true, false],
      ['elsewhere', false, false],
    ])
    expect(profiles[1].path).toBe(
      join(ROOT, 'Profiles', 'q6flhfhs.default-release'),
    )
    expect(profiles[3].path).toBe(normalize('/other/place'))
  })

  test('install defaults beat the legacy flag; the most recently used wins', () => {
    const usedAt = (p: { name: string }) =>
      p.name === 'default-release' ? 2 : 1
    expect(pickDefaultProfile(profiles, usedAt)?.name).toBe('default-release')
    const devFirst = (p: { name: string }) =>
      p.name === 'dev-edition-default' ? 2 : 1
    expect(pickDefaultProfile(profiles, devFirst)?.name).toBe(
      'dev-edition-default',
    )
  })

  test('falls back to the legacy default without install sections', () => {
    const legacy = parseProfilesIni(
      '[Profile0]\nName=a\nPath=a\n[Profile1]\nName=b\nPath=b\nDefault=1\n',
      ROOT,
    )
    expect(pickDefaultProfile(legacy)?.name).toBe('b')
  })

  test('resolves by name or directory name', () => {
    expect(resolveProfile('dev-edition-default', profiles)?.name).toBe(
      'dev-edition-default',
    )
    expect(resolveProfile('6vaq0wfi.default', profiles)?.name).toBe('default')
    expect(resolveProfile('missing', profiles)).toBeUndefined()
  })
})

test('reads the CDP port neo persisted in Local State', () => {
  expect(
    cdpPortFromLocalState('{"browseros":{"server":{"cdp_port":9111}}}'),
  ).toBe(9111)
  expect(cdpPortFromLocalState('{"browseros":{}}')).toBeUndefined()
  expect(cdpPortFromLocalState('{}')).toBeUndefined()
})
