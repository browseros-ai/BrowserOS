import { expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PNG } from 'pngjs'
import { compareImages } from './screenshots.mjs'

const image = (width, height, value = 120) => {
  const png = new PNG({ width, height })
  png.data.fill(value)
  return {
    name: 'sample',
    format: 'png',
    base64: PNG.sync.write(png).toString('base64'),
  }
}
test('screenshots require matching dimensions and retain pixel differences', () => {
  const dir = mkdtempSync(join(tmpdir(), 'neo-pixels-'))
  try {
    expect(compareImages([image(2, 2)], [image(2, 2)], dir).ok).toBe(true)
    expect(compareImages([image(2, 2)], [image(3, 2)], dir).ok).toBe(false)
    expect(compareImages([image(2, 2)], [image(2, 2, 124)], dir).ok).toBe(false)
    expect(compareImages([], [], dir).ok).toBe(false)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
