import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { readSoul, resetSoulTemplate, writeSoul } from '../../lib/soul'

const WriteSoulSchema = z.object({
  content: z.string(),
})

export function createSoulRoutes() {
  return new Hono()
    .get('/', async (c) => {
      const content = await readSoul()
      return c.json({ content })
    })
    .put('/', zValidator('json', WriteSoulSchema), async (c) => {
      const { content } = c.req.valid('json')
      const result = await writeSoul(content)
      return c.json(result)
    })
    .delete('/', async (c) => {
      const result = await resetSoulTemplate()
      return c.json(result)
    })
}
