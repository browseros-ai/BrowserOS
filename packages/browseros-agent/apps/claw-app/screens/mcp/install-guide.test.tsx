import { describe, expect, it } from 'bun:test'
import {
  EXTENSION_DOWNLOAD_URL,
  EXTENSION_RELEASES_URL,
  INSTALL_STEPS,
  MCPB_FILENAME,
} from './install-guide.data'

describe('install-guide data', () => {
  it('walks the six Claude Desktop steps in order', () => {
    expect(INSTALL_STEPS.map((step) => step.id)).toEqual([
      'open-settings',
      'open-extensions',
      'download',
      'install-extension',
      'choose-file',
      'confirm-install',
    ])
  })

  it('gives every step a unique id, a rail title, and an instruction', () => {
    const ids = INSTALL_STEPS.map((step) => step.id)
    expect(new Set(ids).size).toBe(ids.length)
    for (const step of INSTALL_STEPS) {
      expect(step.title.length).toBeGreaterThan(0)
      expect(step.body.length).toBeGreaterThan(0)
    }
  })

  it('reserves the download CTA for exactly one imageless step', () => {
    const downloadSteps = INSTALL_STEPS.filter(
      (step) => step.kind === 'download',
    )
    expect(downloadSteps).toHaveLength(1)
    expect(downloadSteps[0]?.image).toBeUndefined()
    expect(INSTALL_STEPS.filter((step) => step.image === undefined)).toEqual(
      downloadSteps,
    )
  })

  it('serves every step screenshot as webp from a single remote origin', () => {
    const sources: string[] = []
    for (const step of INSTALL_STEPS) {
      if (!step.image) continue
      expect(step.image.alt.length).toBeGreaterThan(0)
      expect(step.image.src.endsWith('.webp')).toBe(true)
      sources.push(step.image.src)
    }
    expect(sources).toHaveLength(5)
    // One origin for all of them, so the bucket is repointable in one edit.
    const origins = new Set(sources.map((src) => new URL(src).origin))
    expect(origins).toEqual(
      new Set(['https://pub-c94be9094f01420f9166e717fbd4a20d.r2.dev']),
    )
  })

  it('pins the direct .mcpb release asset and keeps the picker step in sync', () => {
    expect(EXTENSION_DOWNLOAD_URL).toBe(
      'https://github.com/browseros-ai/browserclaw-claude-desktop/releases/download/v0.5.0/browseros-neo-0.5.0.mcpb',
    )
    expect(EXTENSION_DOWNLOAD_URL.endsWith('.mcpb')).toBe(true)
    expect(MCPB_FILENAME).toBe('browseros-neo-0.5.0.mcpb')
    expect(
      INSTALL_STEPS.find((step) => step.id === 'choose-file')?.body,
    ).toContain(MCPB_FILENAME)
    expect(EXTENSION_RELEASES_URL.endsWith('/releases')).toBe(true)
  })
})
