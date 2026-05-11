#!/usr/bin/env bun
/**
 * Generates extension toolbar/action icon PNGs from the repo root `red_logo.png`.
 * Letterboxes the image on a transparent square (fits full wordmark + mark).
 * Run: `bun scripts/gen-brand-icons.ts`
 */
import { mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import sharp from 'sharp'

const APP = join(dirname(fileURLToPath(import.meta.url)), '..')
const PROJECT_ROOT = join(APP, '..', '..', '..', '..')
const sourcePngPath = join(PROJECT_ROOT, 'red_logo.png')
const iconDir = join(APP, 'public', 'icon')
const brandDir = join(APP, 'public', 'brand')

mkdirSync(iconDir, { recursive: true })
mkdirSync(brandDir, { recursive: true })

const base = sharp(sourcePngPath).ensureAlpha()

for (const size of [16, 32, 48, 96, 128] as const) {
  await base
    .clone()
    .resize(size, size, {
      fit: 'contain',
      position: 'centre',
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    })
    .png()
    .toFile(join(iconDir, `${size}.png`))
}

await base
  .clone()
  .png()
  .toFile(join(brandDir, 'wordmark.png'))

process.stdout.write(
  `Wrote ${[16, 32, 48, 96, 128].map((s) => `icon/${s}.png`).join(', ')}, brand/wordmark.png from red_logo.png\n`,
)
