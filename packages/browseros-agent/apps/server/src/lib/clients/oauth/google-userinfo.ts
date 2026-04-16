/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Google OAuth Helper - ดึงข้อมูลผู้ใช้หลังล็อกอิน Google สำเร็จ
 */

import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'

export interface GoogleUserInfo {
  sub: string
  email: string
  name: string
  picture?: string
}

// ดึงข้อมูลผู้ใช้จาก Google userinfo endpoint
export async function fetchGoogleUserInfo(
  accessToken: string,
): Promise<GoogleUserInfo | null> {
  try {
    const res = await fetch(EXTERNAL_URLS.GOOGLE_USERINFO, {
      headers: { Authorization: `Bearer ${accessToken}` },
    })
    if (!res.ok) return null
    return (await res.json()) as GoogleUserInfo
  } catch {
    return null
  }
}
