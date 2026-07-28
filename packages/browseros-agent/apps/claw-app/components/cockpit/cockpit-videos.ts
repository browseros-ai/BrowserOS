/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Content manifest for the first-run how-to video bento. Editing the
 * line-up here never touches layout: the bento maps `span` to grid cells,
 * derives the poster from the YouTube thumbnail, and plays a self-hosted
 * video in the lightbox (a YouTube iframe cannot play from the extension's
 * chrome-extension origin).
 *
 * Placeholder line-up: two BrowserOS videos fill the grid. Every tile plays
 * the same holdover clip for now; real per-video `videoUrl`s and branded
 * `poster`s drop in later without touching layout.
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
  /** Optional self-hosted video URL; falls back to the shared holdover clip. */
  videoUrl?: string
}

/**
 * Temporary self-hosted clip every tile plays until per-video `videoUrl`s
 * exist. This is the previous single first-run demo.
 */
const PLACEHOLDER_VIDEO_URL =
  'https://cdn.browseros.com/artifacts/claw/onboarding-video/v0.2.0/first-run-demo.mp4'

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

export function videoUrlFor(video: OnboardingVideo): string {
  return video.videoUrl ?? PLACEHOLDER_VIDEO_URL
}
