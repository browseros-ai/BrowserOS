import { afterEach, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { configureKeys, DEFAULT_KEY_DIR } from './keys'

const dirs: string[] = []
afterEach(async () => {
  await Promise.all(
    dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })),
  )
})
/** A home of its own, so no test sees the owner's real key files. */
async function home(withKeys: string[] = []) {
  const dir = await mkdtemp(join(tmpdir(), 'captcha-keys-home-'))
  dirs.push(dir)
  await mkdir(join(dir, DEFAULT_KEY_DIR), { recursive: true })
  for (const name of withKeys)
    await writeFile(join(dir, DEFAULT_KEY_DIR, name), 'fake-private-value')
  return dir
}

test('tests that placeholder keys are private, clear inherited keys and are removed after cleanup', async () => {
  const keys = await configureKeys(
    'placeholder',
    {
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '/owner/real-key',
      BROWSEROS_2CAPTCHA_API_KEY_FILE: '/owner/real-2captcha-key',
    },
    await home(['capsolver-api-key', '2captcha-api-key']),
  )
  const paths = Object.values(keys.env).filter(
    (p): p is string => typeof p === 'string',
  )
  try {
    expect(paths).toHaveLength(1)
    expect(keys.env.BROWSEROS_2CAPTCHA_API_KEY_FILE).toBeUndefined()
    expect(Object.hasOwn(keys.env, 'BROWSEROS_2CAPTCHA_API_KEY_FILE')).toBe(
      true,
    )
    for (const path of paths) {
      expect(path).not.toContain('/owner/')
      expect((await stat(path)).mode & 0o777).toBe(0o600)
      expect(await Bun.file(path).text()).toContain('placeholder')
    }
  } finally {
    await keys.cleanup()
  }
  for (const path of paths) expect(await Bun.file(path).exists()).toBe(false)
  await keys.cleanup()
})

test('tests that none clears every inherited credential', async () => {
  const none = await configureKeys(
    'none',
    {
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '/existing',
      BROWSEROS_OTHER_API_KEY_FILE: '/existing-backup',
    },
    await home(['capsolver-api-key']),
  )
  expect(none.env).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: undefined,
    BROWSEROS_OTHER_API_KEY_FILE: undefined,
  })
  await none.cleanup()
})

test('tests that real keys expand configured paths and fall back to the default folder without reading', async () => {
  const configured = await configureKeys(
    'real',
    {
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '~/missing-key',
      BROWSEROS_2CAPTCHA_API_KEY_FILE: '~/missing-2captcha-key',
      BROWSEROS_OTHER_API_KEY_FILE: '~/missing-backup-key',
    },
    '/home/owner',
  )
  expect(configured.env).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: '/home/owner/missing-key',
    BROWSEROS_2CAPTCHA_API_KEY_FILE: '/home/owner/missing-2captcha-key',
    BROWSEROS_OTHER_API_KEY_FILE: '~/missing-backup-key',
  })
  const owner = await home(['2captcha-api-key'])
  const defaults = await configureKeys('real', {}, owner)
  expect(defaults.env).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: join(
      owner,
      DEFAULT_KEY_DIR,
      'capsolver-api-key',
    ),
    BROWSEROS_2CAPTCHA_API_KEY_FILE: join(
      owner,
      DEFAULT_KEY_DIR,
      '2captcha-api-key',
    ),
  })
  const without2captcha = await configureKeys('real', {}, await home())
  expect(
    Object.hasOwn(without2captcha.env, 'BROWSEROS_2CAPTCHA_API_KEY_FILE'),
  ).toBe(false)
})

test('tests that an additional key path is forwarded without reading or deleting its file', async () => {
  const directory = await home()
  const path = join(directory, 'other-api-key')
  await writeFile(path, 'fake-backup', { mode: 0o600 })
  const keys = await configureKeys(
    'real',
    { BROWSEROS_OTHER_API_KEY_FILE: path },
    directory,
  )
  expect(keys.env.BROWSEROS_OTHER_API_KEY_FILE).toBe(path)
  await keys.cleanup()
  expect(await Bun.file(path).exists()).toBe(true)
})
