/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock, spyOn } from 'bun:test'
import type { ContainerCli } from '../../../src/lib/container/container-cli'
import { ImageLoader } from '../../../src/lib/container/image-loader'
import { ContainerCliError, ImageLoadError } from '../../../src/lib/vm/errors'
import type { VmManifest } from '../../../src/lib/vm/manifest'
import * as paths from '../../../src/lib/vm/paths'

const manifest: VmManifest = {
  schemaVersion: 2,
  updatedAt: '2026-04-22T00:00:00.000Z',
  agents: {
    openclaw: {
      image: 'ghcr.io/openclaw/openclaw',
      version: '2026.4.12',
      tarballs: {
        arm64: {
          key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
          sha256: 'agent-arm',
          sizeBytes: 1,
        },
        x64: {
          key: 'vm/images/openclaw-2026.4.12-x64.tar.gz',
          sha256: 'agent-x64',
          sizeBytes: 1,
        },
      },
    },
  },
}

describe('ImageLoader', () => {
  afterEach(() => {
    mock.restore()
  })

  it('returns without loading when the image already exists', async () => {
    const cli = new FakeContainerCli([true])
    const loader = new ImageLoader(cli as never, manifest, 'arm64')

    await loader.ensureImageLoaded('ghcr.io/openclaw/openclaw:2026.4.12')

    expect(cli.loadCalls).toEqual([])
  })

  it('loads a missing image from the guest cache and verifies it exists', async () => {
    const cli = new FakeContainerCli([false, true])
    const loader = new ImageLoader(cli as never, manifest, 'arm64')

    await loader.ensureImageLoaded('ghcr.io/openclaw/openclaw:2026.4.12')

    expect(cli.loadCalls).toEqual([
      '/mnt/browseros/cache/images/openclaw-2026.4.12-arm64.tar.gz',
    ])
    expect(cli.existsCalls).toEqual([
      'ghcr.io/openclaw/openclaw:2026.4.12',
      'ghcr.io/openclaw/openclaw:2026.4.12',
    ])
  })

  it('resolves image tarballs against the configured BrowserOS root', async () => {
    const cli = new FakeContainerCli([false, true])
    const browserosRoot = '/tmp/browseros-custom-root'
    const loader = new ImageLoader(
      cli as never,
      manifest,
      'arm64',
      browserosRoot,
    )
    const getImageCacheDir = spyOn(paths, 'getImageCacheDir')
    const hostPathToGuest = spyOn(paths, 'hostPathToGuest')

    await loader.ensureImageLoaded('ghcr.io/openclaw/openclaw:2026.4.12')

    expect(getImageCacheDir).toHaveBeenCalledWith(browserosRoot)
    expect(hostPathToGuest).toHaveBeenCalledWith(
      '/tmp/browseros-custom-root/cache/vm/images/openclaw-2026.4.12-arm64.tar.gz',
      browserosRoot,
    )
  })

  it('throws ImageLoadError when a loaded image is still absent', async () => {
    const cli = new FakeContainerCli([false, false])
    const loader = new ImageLoader(cli as never, manifest, 'arm64')

    await expect(
      loader.ensureImageLoaded('ghcr.io/openclaw/openclaw:2026.4.12'),
    ).rejects.toThrow(ImageLoadError)
  })

  it('throws ImageLoadError for unknown refs without loading', async () => {
    const cli = new FakeContainerCli([false])
    const loader = new ImageLoader(cli as never, manifest, 'arm64')

    await expect(loader.ensureImageLoaded('missing:v1')).rejects.toThrow(
      ImageLoadError,
    )
    expect(cli.loadCalls).toEqual([])
  })

  it('wraps ContainerCliError load failures as ImageLoadError', async () => {
    const cli = new FakeContainerCli([false])
    cli.loadError = new ContainerCliError('nerdctl load', 125, 'bad archive')
    const loader = new ImageLoader(cli as never, manifest, 'arm64')

    const error = await loader
      .ensureImageLoaded('ghcr.io/openclaw/openclaw:2026.4.12')
      .catch((err) => err)

    expect(error).toBeInstanceOf(ImageLoadError)
    expect(error.cause).toBe(cli.loadError)
  })
})

class FakeContainerCli
  implements Pick<ContainerCli, 'imageExists' | 'loadImage'>
{
  existsCalls: string[] = []
  loadCalls: string[] = []
  loadError: Error | null = null

  constructor(private readonly existsResponses: boolean[]) {}

  async imageExists(ref: string): Promise<boolean> {
    this.existsCalls.push(ref)
    return this.existsResponses.shift() ?? false
  }

  async loadImage(path: string): Promise<string[]> {
    this.loadCalls.push(path)
    if (this.loadError) throw this.loadError
    return ['loaded']
  }
}
