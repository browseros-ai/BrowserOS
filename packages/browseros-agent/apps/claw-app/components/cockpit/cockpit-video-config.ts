/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit "Learn BrowserClaw" video lineup is controlled from PostHog remote
 * config (flag key `cockpit-videos`), never the app bundle. The payload is an
 * untrusted JSON object; the client validates it, derives each YouTube thumbnail
 * on the fly, and links out to YouTube (a YouTube iframe cannot play from the
 * extension's chrome-extension origin).
 *
 * Payload shape:
 *   {
 *     "heading": "Learn BrowserClaw",            // optional section title
 *     "videos": [
 *       { "url": "https://youtu.be/AF05B1O9nq8", // required YouTube url
 *         "title": "...",                        // optional
 *         "channel": "...",                      // optional
 *         "thumbnail": "https://..." }           // optional poster override
 *     ]
 *   }
 */

import { z } from 'zod'

/** PostHog remote-config flag whose JSON payload holds the video lineup. */
export const COCKPIT_VIDEOS_FLAG = 'cockpit-videos'

export const DEFAULT_HEADING = 'Learn BrowserClaw'

const rawVideoSchema = z.object({
  url: z.string(),
  title: z.string().optional(),
  channel: z.string().optional(),
  thumbnail: z.string().optional(),
})

export const cockpitVideoConfigSchema = z.object({
  heading: z.string().optional(),
  videos: z.array(rawVideoSchema),
})

export type CockpitVideoConfigInput = z.infer<typeof cockpitVideoConfigSchema>

/** A validated, render-ready video (its URL yielded a real YouTube id). */
export interface CockpitVideo {
  id: string
  url: string
  title?: string
  channel?: string
  poster: string
}

export interface CockpitVideoLineup {
  heading: string
  videos: CockpitVideo[]
}

const YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/

/** Extracts the 11-char video id from any common YouTube URL form, else null. */
export function parseYouTubeId(url: string): string | null {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  const host = parsed.hostname.replace(/^www\./, '')
  let id: string | null = null
  if (host === 'youtu.be') {
    id = parsed.pathname.split('/').filter(Boolean)[0] ?? null
  } else if (host === 'youtube.com' || host === 'm.youtube.com') {
    id =
      parsed.pathname === '/watch'
        ? parsed.searchParams.get('v')
        : (parsed.pathname.match(/^\/(?:embed|shorts|v|live)\/([^/]+)/)?.[1] ??
          null)
  }
  return id && YOUTUBE_ID.test(id) ? id : null
}

/** Max-res YouTube thumbnail; the tile falls back to hq on error. */
export function youtubeThumbnail(id: string): string {
  return `https://i.ytimg.com/vi/${id}/maxresdefault.jpg`
}

/** Standard fallback thumbnail; maxres does not exist for every video. */
export function youtubeThumbnailFallback(id: string): string {
  return `https://i.ytimg.com/vi/${id}/hqdefault.jpg`
}

/**
 * Validates an untrusted PostHog payload into a render-ready lineup. A malformed
 * payload, or a video whose URL is not a recognizable YouTube link, is dropped;
 * returns an empty lineup when the payload is unusable.
 */
export function parseCockpitVideoLineup(payload: unknown): CockpitVideoLineup {
  const result = cockpitVideoConfigSchema.safeParse(payload)
  if (!result.success) {
    return { heading: DEFAULT_HEADING, videos: [] }
  }
  const videos: CockpitVideo[] = []
  for (const raw of result.data.videos) {
    const id = parseYouTubeId(raw.url)
    if (!id) continue
    videos.push({
      id,
      url: raw.url,
      title: raw.title,
      channel: raw.channel,
      poster: raw.thumbnail ?? youtubeThumbnail(id),
    })
  }
  return { heading: result.data.heading?.trim() || DEFAULT_HEADING, videos }
}
