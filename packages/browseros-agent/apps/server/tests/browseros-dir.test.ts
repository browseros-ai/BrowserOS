/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import {
  ensureBrowserosDir,
  getBrowserosDir,
  getCacheDir,
  getDbPath,
  getSessionsDir,
  getToolOutputDir,
  logDevelopmentBrowserosDir,
  TOOL_OUTPUT_DIR_MODE,
  writeToolOutputFile,
} from '../src/lib/browseros-dir'
import { logger } from '../src/lib/logger'

describe('getBrowserosDir', () => {
  const originalNodeEnv = process.env.NODE_ENV
  const originalBrowserosDir = process.env.BROWSEROS_DIR

  beforeEach(() => {
    delete process.env.NODE_ENV
    delete process.env.BROWSEROS_DIR
  })

  afterEach(() => {
    if (originalNodeEnv === undefined) {
      delete process.env.NODE_ENV
    } else {
      process.env.NODE_ENV = originalNodeEnv
    }

    if (originalBrowserosDir === undefined) {
      delete process.env.BROWSEROS_DIR
    } else {
      process.env.BROWSEROS_DIR = originalBrowserosDir
    }
  })

  it('uses a separate home directory in development', () => {
    process.env.NODE_ENV = 'development'

    expect(getBrowserosDir()).toBe(join(homedir(), '.browseros-dev'))
  })

  it('uses the standard home directory outside development', () => {
    process.env.NODE_ENV = 'test'

    expect(getBrowserosDir()).toBe(join(homedir(), PATHS.BROWSEROS_DIR_NAME))
  })

  it('logs the resolved development directory path', () => {
    process.env.NODE_ENV = 'development'
    const originalInfo = logger.info
    const info = mock(() => {})
    logger.info = info

    try {
      logDevelopmentBrowserosDir()

      expect(info).toHaveBeenCalledWith(
        `Using development BrowserOS directory: ${join(homedir(), '.browseros-dev')}`,
      )
    } finally {
      logger.info = originalInfo
    }
  })

  it('does not log a development directory outside development', () => {
    process.env.NODE_ENV = 'test'
    const originalInfo = logger.info
    const info = mock(() => {})
    logger.info = info

    try {
      logDevelopmentBrowserosDir()

      expect(info).not.toHaveBeenCalled()
    } finally {
      logger.info = originalInfo
    }
  })

  it('uses the development cache directory in development', () => {
    process.env.NODE_ENV = 'development'

    expect(getCacheDir()).toBe(join(homedir(), '.browseros-dev', 'cache'))
  })

  it('uses the BrowserOS directory for the sqlite database', () => {
    process.env.NODE_ENV = 'development'

    expect(getDbPath()).toBe(
      join(
        homedir(),
        PATHS.DEV_BROWSEROS_DIR_NAME,
        PATHS.DB_DIR_NAME,
        PATHS.DB_FILE_NAME,
      ),
    )
  })

  it('uses the standard BrowserOS directory for the sqlite database outside development', () => {
    process.env.NODE_ENV = 'test'

    expect(getDbPath()).toBe(
      join(
        homedir(),
        PATHS.BROWSEROS_DIR_NAME,
        PATHS.DB_DIR_NAME,
        PATHS.DB_FILE_NAME,
      ),
    )
  })

  it('uses the standard cache directory outside development', () => {
    process.env.NODE_ENV = 'test'

    expect(getCacheDir()).toBe(
      join(homedir(), PATHS.BROWSEROS_DIR_NAME, 'cache'),
    )
  })
  it('creates only the startup-owned directories during startup setup', async () => {
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-dir-test-'))
    process.env.BROWSEROS_DIR = browserosDir

    try {
      await ensureBrowserosDir()

      expect(existsSync(getSessionsDir())).toBe(true)
      expect(existsSync(join(browserosDir, 'tool-output'))).toBe(true)
      expect(existsSync(join(browserosDir, 'cache', 'vm'))).toBe(false)
      expect(existsSync(join(browserosDir, 'vm'))).toBe(false)
      expect(existsSync(join(browserosDir, 'lazy-monitoring'))).toBe(false)
      expect(existsSync(join(browserosDir, 'lazy-monitoring', 'runs'))).toBe(
        false,
      )
    } finally {
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })

  it('locks down the tool output directory permissions', async () => {
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-dir-test-'))
    process.env.BROWSEROS_DIR = browserosDir

    try {
      const rawOutputDir = join(browserosDir, 'tool-output')
      const createdOutputDir = await getToolOutputDir()
      expect(createdOutputDir).toBe(realpathSync(rawOutputDir))
      if (process.platform !== 'win32') {
        chmodSync(rawOutputDir, 0o777)
      }

      const outputDir = await getToolOutputDir()

      expect(outputDir).toBe(realpathSync(rawOutputDir))
      if (process.platform !== 'win32') {
        expect(statSync(outputDir).mode & 0o777).toBe(TOOL_OUTPUT_DIR_MODE)
      }
    } finally {
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })

  it('does not overwrite existing generated tool output files', async () => {
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-dir-test-'))
    process.env.BROWSEROS_DIR = browserosDir

    try {
      const outputDir = await getToolOutputDir()
      const outputPath = join(outputDir, 'existing.txt')
      writeFileSync(outputPath, 'original')

      await expect(
        writeToolOutputFile(outputPath, 'replacement'),
      ).rejects.toThrow('EEXIST')
    } finally {
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })
})
