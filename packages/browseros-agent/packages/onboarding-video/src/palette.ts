/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Palette tokens that mirror the claw-app cockpit CSS variables so
 * the Remotion demo reads as a scene from the actual product. When
 * the app palette shifts, update these here in one place; every
 * scene consumes tokens by name.
 */

export const palette = {
  accent: '#0254ec',
  accentTint: '#e0ecff',
  ink: '#0a0d14',
  ink2: '#3b4152',
  ink3: '#6b7285',
  bgCanvas: '#f6f8fb',
  bgSunken: '#eef1f6',
  card: '#ffffff',
  cardTint: '#f2f4f8',
  border2: '#d9dee6',
  green: '#0f9968',
  amber: '#c47600',
} as const

export type PaletteToken = keyof typeof palette
