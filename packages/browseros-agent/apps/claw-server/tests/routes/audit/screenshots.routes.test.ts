/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import app from '../../../src/server'
import { screenshotPath } from '../../../src/services/screenshots'
import { withTempBrowserClawDir } from '../../_helpers/temp-browserclaw-dir'

describe('GET /audit/screenshot/:dispatchId', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('400 on non-integer id', async () => {
    const res = await app.fetch(
      new Request('http://localhost/audit/screenshot/abc'),
    )
    expect(res.status).toBe(400)
  })

  it('404 when the file is not on disk', async () => {
    await withTempBrowserClawDir(async () => {
      const res = await app.fetch(
        new Request('http://localhost/audit/screenshot/9999'),
      )
      expect(res.status).toBe(404)
    })
  })

  it('streams the file with image/jpeg content-type', async () => {
    await withTempBrowserClawDir(async () => {
      const path = screenshotPath(7)
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, Buffer.from([0xff, 0xd8, 0xff, 0xe0]))
      const res = await app.fetch(
        new Request('http://localhost/audit/screenshot/7'),
      )
      expect(res.status).toBe(200)
      expect(res.headers.get('content-type')).toBe('image/jpeg')
      const ab = await res.arrayBuffer()
      expect(ab.byteLength).toBe(4)
    })
  })
})
