import { mkdir, rm } from 'node:fs/promises'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { getCoreMemoryPath, getMemoryDir } from '../../lib/browseros-dir'

const MAX_CONTENT_LENGTH = 50_000

const SaveMemorySchema = z.object({
  content: z.string().max(MAX_CONTENT_LENGTH),
})

export function createMemoryRoutes() {
  return new Hono()
    .get('/', async (c) => {
      const file = Bun.file(getCoreMemoryPath())
      if (!(await file.exists())) {
        return c.json({ content: '' })
      }
      const content = await file.text()
      return c.json({ content })
    })
    .put('/', zValidator('json', SaveMemorySchema), async (c) => {
      const { content } = c.req.valid('json')
      await mkdir(getMemoryDir(), { recursive: true })
      await Bun.write(getCoreMemoryPath(), content)
      return c.json({ success: true })
    })
    .delete('/', async (c) => {
      await rm(getMemoryDir(), { recursive: true, force: true })
      await mkdir(getMemoryDir(), { recursive: true })
      return c.json({ success: true })
    })
}
