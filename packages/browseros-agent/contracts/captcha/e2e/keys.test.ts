import { expect, test } from 'bun:test'
import { stat } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { configureKeys } from './keys'

test('tests that placeholder keys are private and removed after cleanup', async () => {
  const keys = await configureKeys('placeholder')
  const paths = Object.values(keys.env).filter(
    (p): p is string => typeof p === 'string',
  )
  expect(paths).toHaveLength(2)
  try {
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
  })
  expect(none.env).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: undefined,
    BROWSEROS_CAPMONSTER_API_KEY_FILE: undefined,
  })
  expect(Object.keys(none.env)).toEqual([
    'BROWSEROS_CAPSOLVER_API_KEY_FILE',
    'BROWSEROS_CAPMONSTER_API_KEY_FILE',
  ])
  const real = await configureKeys('real', {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: '~/missing-key',
  })
  expect(real.env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe(
    join(homedir(), 'missing-key'),
  )
  expect(real.env.BROWSEROS_CAPMONSTER_API_KEY_FILE).toBe(
    join(homedir(), '.config/browseros/captcha/capmonster-api-key'),
  )
  await none.cleanup()
  await real.cleanup()
})
