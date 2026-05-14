import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  createSkillSource,
  deleteSkillSource,
  listSkillSources,
  updateSkillSource,
} from '../../skills/service'

const CreateSourceSchema = z.object({
  id: z.string().min(1).max(100),
  path: z.string().min(1),
  enabled: z.boolean().default(true),
  label: z.string().max(100).optional(),
})

const UpdateSourceSchema = z.object({
  path: z.string().min(1).optional(),
  enabled: z.boolean().optional(),
  label: z.string().max(100).optional(),
})

export function createSkillSourcesRoutes() {
  return new Hono()
    .get('/', async (c) => {
      const sources = await listSkillSources()
      return c.json({ sources })
    })
    .post('/', zValidator('json', CreateSourceSchema), async (c) => {
      try {
        const source = await createSkillSource(c.req.valid('json'))
        return c.json({ source }, 201)
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to create'
        const status = msg.includes('already exists') ? 409 : 500
        return c.json({ error: msg }, status)
      }
    })
    .put('/:id', zValidator('json', UpdateSourceSchema), async (c) => {
      try {
        const source = await updateSkillSource(
          c.req.param('id'),
          c.req.valid('json'),
        )
        return c.json({ source })
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to update'
        const status = msg.includes('not found') ? 404 : 500
        return c.json({ error: msg }, status)
      }
    })
    .delete('/:id', async (c) => {
      try {
        await deleteSkillSource(c.req.param('id'))
        return c.json({ ok: true })
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to delete'
        const status = msg.includes('not found') ? 404 : 500
        return c.json({ error: msg }, status)
      }
    })
}
