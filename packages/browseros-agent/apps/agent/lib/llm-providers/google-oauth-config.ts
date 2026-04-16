/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Google OAuth client config สำหรับ BrowserOS extension
 * ใช้กับ useOAuthProviderFlow hook
 */

import type { OAuthProviderFlowConfig } from './useOAuthProviderFlow'
import type { ProviderType } from './types'

// Google OAuth flow config สำหรับ hook useOAuthProviderFlow
export const googleOAuthFlowConfig: OAuthProviderFlowConfig = {
  providerType: 'google' as ProviderType,
  displayName: 'Google (Gemini)',
  startedEvent: 'google_oauth_started',
  completedEvent: 'google_oauth_completed',
  disconnectedEvent: 'google_oauth_disconnected',
}
