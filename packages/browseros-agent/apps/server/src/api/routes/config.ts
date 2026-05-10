import {
  CONFIG_KEYS,
  type ConfigOverrides,
} from '@browseros/shared/constants/config-schema'
import { configStore } from '@browseros/shared/constants/config-store'
import { Hono } from 'hono'
import { getAdvancedConfigPath } from '../../lib/browseros-dir'
import type { Env } from '../types'

export function createConfigRoutes() {
  let initializedPath: string | null = null

  function ensureInitialized(): void {
    const nextPath = getAdvancedConfigPath()
    if (initializedPath !== nextPath) {
      configStore.init(nextPath)
      initializedPath = nextPath
    }
  }

  return new Hono<Env>()
    .get('/', (c) => {
      ensureInitialized()

      return c.json({
        active: configStore.getAllActive(),
        pending: configStore.getFileOverrides(),
        defaults: configStore.getDefaults(),
        schema: Object.fromEntries(
          CONFIG_KEYS.map((entry) => [entry.key, entry]),
        ),
        hasPendingChanges: configStore.hasPendingChanges(),
      })
    })
    .put('/', async (c) => {
      ensureInitialized()

      const body = (await c.req.json().catch(() => null)) as {
        overrides?: ConfigOverrides
      } | null

      if (!body?.overrides || typeof body.overrides !== 'object') {
        return c.json(
          {
            ok: false,
            errors: [{ key: '_', message: 'Missing overrides object' }],
          },
          400,
        )
      }

      const errors = configStore.save(body.overrides)
      if (errors.size > 0) {
        return c.json(
          {
            ok: false,
            errors: Array.from(errors.entries()).map(([key, message]) => ({
              key,
              message,
            })),
          },
          400,
        )
      }

      return c.json({
        ok: true,
        saved: Object.keys(body.overrides).length,
        hasPendingChanges: configStore.hasPendingChanges(),
      })
    })
    .delete('/', (c) => {
      ensureInitialized()
      configStore.reset()
      return c.json({ ok: true, hasPendingChanges: false })
    })
}
