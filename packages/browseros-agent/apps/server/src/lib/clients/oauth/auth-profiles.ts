/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * ระบบ Auth Profiles - จัดการการยืนยันตัวตนแบบรวม
 * รองรับทั้ง API Key และ OAuth ในระบบเดียวกัน
 * แต่ละ provider มี profile ของตัวเอง
 */

import type { Database } from 'bun:sqlite'
import { logger } from '../../logger'
import { getOAuthProvider } from './providers'
import type { OAuthTokenManager } from './token-manager'

// ประเภทของการยืนยันตัวตน
export type AuthMethod = 'api-key' | 'oauth'

// สถานะของ auth profile
export type AuthStatus = 'active' | 'expired' | 'disconnected'

// ข้อมูล auth profile สำหรับแต่ละ provider
export interface AuthProfile {
  providerId: string
  providerName: string
  method: AuthMethod
  status: AuthStatus
  email?: string
  accountId?: string
  lastAuthenticated?: number
  expiresAt?: number
}

// ผลลัพธ์การยืนยันตัวตน - ส่งให้ provider ใช้ได้เลย
export interface AuthCredentials {
  type: 'api-key' | 'oauth'
  apiKey?: string
  accessToken?: string
  refreshToken?: string
  expiresAt?: number
}

export class AuthProfileManager {
  constructor(
    private readonly db: Database,
    private readonly browserosId: string,
    private readonly tokenManager?: OAuthTokenManager,
  ) {
    this.ensureTable()
  }

  // สร้างตาราง auth_profiles ถ้ายังไม่มี
  private ensureTable(): void {
    this.db.run(`
      CREATE TABLE IF NOT EXISTS auth_profiles (
        browseros_id TEXT NOT NULL,
        provider_id TEXT NOT NULL,
        method TEXT NOT NULL DEFAULT 'api-key',
        api_key TEXT,
        email TEXT,
        account_id TEXT,
        last_authenticated INTEGER,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch()),
        PRIMARY KEY (browseros_id, provider_id)
      )
    `)
  }

  // ดึง credentials สำหรับ provider (ตรวจสอบ token refresh อัตโนมัติ)
  async getCredentials(providerId: string): Promise<AuthCredentials | null> {
    // ลองดึงจาก OAuth ก่อน
    if (this.tokenManager) {
      const oauthProvider = getOAuthProvider(providerId)
      if (oauthProvider) {
        try {
          const tokens = await this.tokenManager.refreshIfExpired(providerId)
          if (tokens) {
            return {
              type: 'oauth',
              accessToken: tokens.accessToken,
              refreshToken: tokens.refreshToken,
              expiresAt: tokens.expiresAt,
            }
          }
        } catch {
          // Token หมดอายุแล้ว ลองดู API Key
          logger.debug('OAuth token expired, falling back to API key', {
            provider: providerId,
          })
        }
      }
    }

    // ดึงจาก API Key
    const row = this.db
      .prepare(
        'SELECT api_key FROM auth_profiles WHERE browseros_id = ? AND provider_id = ? AND api_key IS NOT NULL',
      )
      .get(this.browserosId, providerId) as { api_key: string } | null

    if (row?.api_key) {
      return { type: 'api-key', apiKey: row.api_key }
    }

    return null
  }

  // บันทึก API Key สำหรับ provider
  saveApiKey(providerId: string, apiKey: string): void {
    this.db.prepare(`
      INSERT INTO auth_profiles (browseros_id, provider_id, method, api_key, last_authenticated, updated_at)
      VALUES (?, ?, 'api-key', ?, unixepoch(), unixepoch())
      ON CONFLICT (browseros_id, provider_id) DO UPDATE SET
        method = 'api-key',
        api_key = excluded.api_key,
        last_authenticated = unixepoch(),
        updated_at = unixepoch()
    `).run(this.browserosId, providerId, apiKey)

    logger.info('API key saved', { provider: providerId })
  }

  // ดึงสถานะ auth profiles ทั้งหมด
  getAllProfiles(): AuthProfile[] {
    const rows = this.db
      .prepare(
        `SELECT provider_id, method, email, account_id, last_authenticated
         FROM auth_profiles WHERE browseros_id = ?`,
      )
      .all(this.browserosId) as Array<{
      provider_id: string
      method: string
      email: string | null
      account_id: string | null
      last_authenticated: number | null
    }>

    const profiles: AuthProfile[] = rows.map((row) => {
      const oauthProvider = getOAuthProvider(row.provider_id)
      let status: AuthStatus = 'active'
      let expiresAt: number | undefined

      // ตรวจสอบสถานะ OAuth
      if (row.method === 'oauth' && this.tokenManager) {
        const tokens = this.tokenManager.getTokens(row.provider_id)
        if (!tokens) {
          status = 'disconnected'
        } else if (tokens.expiresAt > 0 && Date.now() > tokens.expiresAt) {
          status = 'expired'
          expiresAt = tokens.expiresAt
        } else {
          expiresAt = tokens.expiresAt || undefined
        }
      }

      return {
        providerId: row.provider_id,
        providerName: oauthProvider?.name ?? row.provider_id,
        method: row.method as AuthMethod,
        status,
        email: row.email ?? undefined,
        accountId: row.account_id ?? undefined,
        lastAuthenticated: row.last_authenticated ?? undefined,
        expiresAt,
      }
    })

    return profiles
  }

  // ลบ auth profile
  deleteProfile(providerId: string): void {
    this.db
      .prepare(
        'DELETE FROM auth_profiles WHERE browseros_id = ? AND provider_id = ?',
      )
      .run(this.browserosId, providerId)

    // ลบ OAuth tokens ด้วย
    this.tokenManager?.deleteTokens(providerId)
    logger.info('Auth profile deleted', { provider: providerId })
  }

  // อัปเดต email/accountId หลัง OAuth สำเร็จ
  updateOAuthProfile(
    providerId: string,
    email?: string,
    accountId?: string,
  ): void {
    this.db.prepare(`
      INSERT INTO auth_profiles (browseros_id, provider_id, method, email, account_id, last_authenticated, updated_at)
      VALUES (?, ?, 'oauth', ?, ?, unixepoch(), unixepoch())
      ON CONFLICT (browseros_id, provider_id) DO UPDATE SET
        method = 'oauth',
        email = excluded.email,
        account_id = excluded.account_id,
        last_authenticated = unixepoch(),
        updated_at = unixepoch()
    `).run(
      this.browserosId,
      providerId,
      email ?? null,
      accountId ?? null,
    )
  }
}
