/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { renderLimaTemplate } from '../../../src/lib/vm/lima-config'

describe('renderLimaTemplate', () => {
  it('injects BrowserOS host mounts into the bundled Lima template', () => {
    const yaml = renderLimaTemplate(
      'minimumLimaVersion: 2.0.0\nmounts: []\nprobes: []\n',
      {
        vmStateDir: '/Users/me/.browseros/vm',
        imageCacheDir: '/Users/me/.browseros/cache/vm/images',
      },
    )

    expect(yaml).toContain('mountPoint: "/mnt/browseros/vm"')
    expect(yaml).toContain('location: "/Users/me/.browseros/vm"')
    expect(yaml).toContain('mountPoint: "/mnt/browseros/cache/images"')
    expect(yaml).toContain('location: "/Users/me/.browseros/cache/vm/images"')
    expect(yaml).toContain('probes: []')
  })

  it('fails loudly if the template no longer has the expected mount marker', () => {
    expect(() =>
      renderLimaTemplate('minimumLimaVersion: 2.0.0\n', {
        vmStateDir: '/state',
        imageCacheDir: '/images',
      }),
    ).toThrow('mounts: [] marker')
  })
})
