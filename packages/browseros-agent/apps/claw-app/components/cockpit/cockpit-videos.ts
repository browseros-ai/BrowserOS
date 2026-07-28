/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Content manifest for the first-run how-to video bento. Editing the
 * line-up here never touches layout: the bento maps `span` to grid
 * cells and derives the poster and embed URL from `youtubeId`.
 *
 * Placeholder line-up: two BrowserOS videos repeated to fill the grid.
 * Real how-to clips replace these later by swapping `youtubeId` / copy.
 * A per-video `poster` URL can override the derived YouTube thumbnail
 * once branded posters exist.
 */

export type VideoSpan = 'featured' | 'wide' | 'default'

export interface OnboardingVideo {
  /** Stable key for the tile (not the YouTube id, so repeats stay unique). */
  id: string
  youtubeId: string
  title: string
  channel: string
  span: VideoSpan
  /** Optional poster URL; falls back to the YouTube thumbnail. */
  poster?: string
}

const CAN_AI_AUTOMATE = {
  youtubeId: 'rIZ8OBHL7Zo',
  title: 'Can AI Agents Finally Automate the Web?',
  channel: 'Better Stack',
} as const

const FUTURE_OF_BROWSING = {
  youtubeId: 'lUvgw7v-avA',
  title: 'The Future of Browsing Is Here',
  channel: 'Tech With Mobin',
} as const

export const ONBOARDING_VIDEOS: readonly OnboardingVideo[] = [
  { id: 'v1', span: 'featured', ...CAN_AI_AUTOMATE },
  { id: 'v2', span: 'default', ...FUTURE_OF_BROWSING },
  { id: 'v3', span: 'default', ...CAN_AI_AUTOMATE },
  { id: 'v4', span: 'default', ...FUTURE_OF_BROWSING },
  { id: 'v5', span: 'wide', ...CAN_AI_AUTOMATE },
]

export function posterFor(video: OnboardingVideo): string {
  return (
    video.poster ??
    `https://i.ytimg.com/vi/${video.youtubeId}/maxresdefault.jpg`
  )
}

/**
 * Privacy-friendly nocookie embed, autoplaying because the user
 * explicitly clicked the tile to open the player.
 */
export function embedUrlFor(video: OnboardingVideo): string {
  return `https://www.youtube-nocookie.com/embed/${video.youtubeId}?autoplay=1&rel=0`
}
