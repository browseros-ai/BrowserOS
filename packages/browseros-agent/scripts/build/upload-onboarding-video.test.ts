import { afterEach, describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  buildOnboardingVideoUploadPlan,
  parseOnboardingVideoUploadArgs,
  validateOnboardingVideoInputs,
} from './upload-onboarding-video'

describe('onboarding video upload', () => {
  let tempRoot: string | null = null

  afterEach(() => {
    if (tempRoot) {
      rmSync(tempRoot, { recursive: true, force: true })
      tempRoot = null
    }
  })

  test('builds versioned R2 keys and CDN URLs', () => {
    const rootDir = writeOnboardingVideoRoot('0.1.0')
    writeRenderOutput(rootDir, 'first-run-demo.mp4')
    writeRenderOutput(rootDir, 'first-run-demo-poster.png')

    const plan = buildOnboardingVideoUploadPlan(rootDir)

    expect(plan).toEqual({
      version: '0.1.0',
      assets: [
        {
          filename: 'first-run-demo.mp4',
          relativePath: 'packages/onboarding-video/out/first-run-demo.mp4',
          absolutePath: join(
            rootDir,
            'packages/onboarding-video/out/first-run-demo.mp4',
          ),
          contentType: 'video/mp4',
          key: 'artifacts/claw/onboarding-video/v0.1.0/first-run-demo.mp4',
          url: 'https://cdn.browseros.com/artifacts/claw/onboarding-video/v0.1.0/first-run-demo.mp4',
          renderCommand: 'bun run --cwd packages/onboarding-video render',
        },
        {
          filename: 'first-run-demo-poster.png',
          relativePath:
            'packages/onboarding-video/out/first-run-demo-poster.png',
          absolutePath: join(
            rootDir,
            'packages/onboarding-video/out/first-run-demo-poster.png',
          ),
          contentType: 'image/png',
          key: 'artifacts/claw/onboarding-video/v0.1.0/first-run-demo-poster.png',
          url: 'https://cdn.browseros.com/artifacts/claw/onboarding-video/v0.1.0/first-run-demo-poster.png',
          renderCommand:
            'bun run --cwd packages/onboarding-video render:poster',
        },
      ],
    })
    expect(() => validateOnboardingVideoInputs(plan)).not.toThrow()
  })

  test('reports missing renders with exact render commands', () => {
    const rootDir = writeOnboardingVideoRoot('0.1.0')
    writeRenderOutput(rootDir, 'first-run-demo.mp4')
    const plan = buildOnboardingVideoUploadPlan(rootDir)

    expect(() => validateOnboardingVideoInputs(plan)).toThrow(
      [
        'Missing onboarding video render output:',
        '- packages/onboarding-video/out/first-run-demo-poster.png',
        '',
        'Render the missing assets first:',
        '  bun run --cwd packages/onboarding-video render:poster',
      ].join('\n'),
    )
  })

  test('parses dry-run and force flags', () => {
    expect(
      parseOnboardingVideoUploadArgs(['--', '--dry-run', '--force']),
    ).toEqual({
      dryRun: true,
      force: true,
    })
    expect(() => parseOnboardingVideoUploadArgs(['--unknown'])).toThrow(
      'Unknown option: --unknown',
    )
  })

  function writeOnboardingVideoRoot(version: string): string {
    tempRoot = mkdtempSync(join(tmpdir(), 'onboarding-video-upload-'))
    const packageDir = join(tempRoot, 'packages/onboarding-video')
    mkdirSync(packageDir, { recursive: true })
    writeFileSync(
      join(packageDir, 'package.json'),
      `${JSON.stringify({ version })}\n`,
    )
    return tempRoot
  }
})

function writeRenderOutput(rootDir: string, filename: string): void {
  const outDir = join(rootDir, 'packages/onboarding-video/out')
  mkdirSync(outDir, { recursive: true })
  writeFileSync(join(outDir, filename), 'stub')
}
