import { describe, expect, it } from 'bun:test'

import { parseBuildArgs } from '../src'
import { testProduct } from './helpers'

describe('build CLI', () => {
  it('uploads by default for server-compatible products', () => {
    const args = parseBuildArgs(['--target=darwin-arm64'], testProduct())

    expect(args.upload).toBe(true)
  })

  it('honors products that build local artifacts by default', () => {
    const args = parseBuildArgs(
      ['--target=darwin-arm64'],
      testProduct({ defaultUpload: false }),
    )

    expect(args.upload).toBe(false)
  })

  it('lets --upload override a local-by-default product', () => {
    const args = parseBuildArgs(
      ['--target=darwin-arm64', '--upload'],
      testProduct({ defaultUpload: false }),
    )

    expect(args.upload).toBe(true)
  })
})
