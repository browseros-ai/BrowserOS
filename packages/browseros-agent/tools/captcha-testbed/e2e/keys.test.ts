import { expect, test } from 'bun:test'
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { configureKeys } from './keys'

test('tests that placeholder keys are private and removed after cleanup', async () => {
  const keys = await configureKeys('placeholder', {
    BROWSEROS_OTHER_API_KEY_FILE: '/existing-backup',
  })
  const paths = Object.values(keys.env).filter(
    (p): p is string => typeof p === 'string',
  )
  try {
    expect(paths).toHaveLength(1)
    expect(Object.hasOwn(keys.env, 'BROWSEROS_OTHER_API_KEY_FILE')).toBe(true)
    expect(keys.env.BROWSEROS_OTHER_API_KEY_FILE).toBeUndefined()
    for (const path of paths) {
      expect((await stat(path)).mode & 0o777).toBe(0o600)
      expect(await Bun.file(path).text()).toContain('placeholder')
    }
  } finally {
    await keys.cleanup()
  }
  for (const path of paths) expect(await Bun.file(path).exists()).toBe(false)
  await keys.cleanup()
})

test('tests that none clears inherited credentials and real expands paths without reading them', async () => {
  const none = await configureKeys('none', {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: '/existing',
    BROWSEROS_OTHER_API_KEY_FILE: '/existing-backup',
  })
  expect(none.env).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: undefined,
    BROWSEROS_OTHER_API_KEY_FILE: undefined,
  })
  expect(Object.keys(none.env)).toEqual([
    'BROWSEROS_CAPSOLVER_API_KEY_FILE',
    'BROWSEROS_OTHER_API_KEY_FILE',
  ])
  const real = await configureKeys('real', {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: '~/missing-key',
    BROWSEROS_OTHER_API_KEY_FILE: '~/missing-backup-key',
  })
  expect(real.env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe(
    join(homedir(), 'missing-key'),
  )
  expect(real.env.BROWSEROS_OTHER_API_KEY_FILE).toBe('~/missing-backup-key')
  const defaults = await configureKeys('real', {})
  expect(defaults.env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe(
    join(homedir(), '.config/browseros/captcha/capsolver-api-key'),
  )
  await none.cleanup()
  await real.cleanup()
  await defaults.cleanup()
})

test('tests that an additional key path is forwarded without reading or deleting its file', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'captcha-backup-test-'))
  const path = join(directory, 'other-api-key')
  await writeFile(path, 'fake-backup', { mode: 0o600 })
  try {
    const keys = await configureKeys('real', {
      BROWSEROS_OTHER_API_KEY_FILE: path,
    })
    expect(keys.env.BROWSEROS_OTHER_API_KEY_FILE).toBe(path)
    await keys.cleanup()
    expect(await Bun.file(path).exists()).toBe(true)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
