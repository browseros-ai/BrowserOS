/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * API routes สำหรับระบบ Auth Profiles
 * จัดการ API Key + OAuth ในที่เดียว
 */

import { Hono } from 'hono'
import type { AuthProfileManager } from '../../lib/clients/oauth/auth-profiles'
import { logger } from '../../lib/logger'

interface AuthProfileRoutesDeps {
  profileManager: AuthProfileManager
}

export function createAuthProfileRoutes(deps: AuthProfileRoutesDeps) {
  const { profileManager } = deps

  return new Hono()
    // ดึง auth profiles ทั้งหมด
    .get('/', (c) => {
      const profiles = profileManager.getAllProfiles()
      return c.json(profiles)
    })

    // ดึง credentials สำหรับ provider ใดโดยเฉพาะ
    .get('/:provider/credentials', async (c) => {
      const providerId = c.req.param('provider')
      const credentials = await profileManager.getCredentials(providerId)
      if (!credentials) {
        return c.json({ error: 'No credentials found' }, 404)
      }
      // ไม่ส่งค่าจริงกลับไป — ส่งแค่ type และสถานะ
      return c.json({
        type: credentials.type,
        hasToken: !!(credentials.accessToken || credentials.apiKey),
        expiresAt: credentials.expiresAt,
      })
    })

    // บันทึก API Key สำหรับ provider
    .post('/:provider/api-key', async (c) => {
      const providerId = c.req.param('provider')
      const body = await c.req.json<{ apiKey?: string }>()

      if (!body.apiKey?.trim()) {
        return c.json({ error: 'API key is required' }, 400)
      }

      profileManager.saveApiKey(providerId, body.apiKey.trim())
      logger.info('API key saved via auth profiles', { provider: providerId })
      return c.json({ success: true })
    })

    // ลบ auth profile (ทั้ง API Key และ OAuth)
    .delete('/:provider', (c) => {
      const providerId = c.req.param('provider')
      profileManager.deleteProfile(providerId)
      logger.info('Auth profile deleted', { provider: providerId })
      return c.json({ success: true })
    })
}
