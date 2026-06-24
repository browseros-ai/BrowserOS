/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Phase 0 placeholder for the v2 onboarding flow. The full design
 * (browser-for-agents-v2) lands in a later phase; this stub keeps
 * `/onboarding` reachable so any future deep link does not 404 and
 * gives the UI a place to render the v2 hero copy in the meantime.
 */

export function OnboardingV2() {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        minHeight: '100vh',
        gap: 12,
        textAlign: 'center',
        padding: '0 24px',
      }}
    >
      <h1 style={{ fontSize: 28, margin: 0 }}>BrowserOS for agents</h1>
      <p style={{ fontSize: 16, color: '#666', maxWidth: 520 }}>
        Connect your AI agent to BrowserOS via the standard MCP endpoint at{' '}
        <code>http://127.0.0.1:9100/cockpit/mcp</code>. The richer onboarding
        flow ships in the next release.
      </p>
    </div>
  )
}
