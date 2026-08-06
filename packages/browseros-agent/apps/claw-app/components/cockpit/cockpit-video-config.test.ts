/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  DEFAULT_HEADING,
  parseCockpitVideoLineup,
  parseYouTubeId,
  youtubeThumbnail,
} from './cockpit-video-config'

describe('parseYouTubeId', () => {
  it('parses the common YouTube URL forms', () => {
    for (const url of [
      'https://www.youtube.com/watch?v=AF05B1O9nq8',
      'https://youtu.be/AF05B1O9nq8',
      'https://www.youtube.com/embed/AF05B1O9nq8',
      'https://www.youtube.com/shorts/AF05B1O9nq8',
      'https://youtube.com/watch?v=AF05B1O9nq8&t=30s',
      'https://m.youtube.com/watch?v=AF05B1O9nq8',
    ]) {
      expect(parseYouTubeId(url)).toBe('AF05B1O9nq8')
    }
  })

  it('rejects non-YouTube, malformed, and wrong-length ids', () => {
    expect(parseYouTubeId('https://vimeo.com/12345')).toBeNull()
    expect(parseYouTubeId('not a url')).toBeNull()
    expect(parseYouTubeId('https://www.youtube.com/watch?v=short')).toBeNull()
    expect(parseYouTubeId('https://www.youtube.com/')).toBeNull()
  })
})

describe('parseCockpitVideoLineup', () => {
  it('validates a payload and derives thumbnails', () => {
    const lineup = parseCockpitVideoLineup({
      heading: 'Watch these',
      videos: [
        {
          url: 'https://youtu.be/AF05B1O9nq8',
          title: 'One',
          channel: 'BrowserOS',
        },
        { url: 'https://www.youtube.com/watch?v=rIZ8OBHL7Zo' },
      ],
    })
    expect(lineup.heading).toBe('Watch these')
    expect(lineup.videos).toHaveLength(2)
    expect(lineup.videos[0]).toMatchObject({
      id: 'AF05B1O9nq8',
      url: 'https://youtu.be/AF05B1O9nq8',
      title: 'One',
      channel: 'BrowserOS',
      poster: youtubeThumbnail('AF05B1O9nq8'),
    })
    expect(lineup.videos[1].id).toBe('rIZ8OBHL7Zo')
  })

  it('honors an explicit thumbnail override', () => {
    const lineup = parseCockpitVideoLineup({
      videos: [
        {
          url: 'https://youtu.be/AF05B1O9nq8',
          thumbnail: 'https://cdn.example.com/p.jpg',
        },
      ],
    })
    expect(lineup.videos[0].poster).toBe('https://cdn.example.com/p.jpg')
  })

  it('drops videos whose url is not a recognizable YouTube link', () => {
    const lineup = parseCockpitVideoLineup({
      videos: [
        { url: 'https://vimeo.com/1' },
        { url: 'https://youtu.be/AF05B1O9nq8' },
      ],
    })
    expect(lineup.videos.map((video) => video.id)).toEqual(['AF05B1O9nq8'])
  })

  it('defaults the heading and returns empty on an unusable payload', () => {
    const empty = { heading: DEFAULT_HEADING, videos: [] }
    expect(parseCockpitVideoLineup({ videos: [] })).toEqual(empty)
    expect(parseCockpitVideoLineup(null)).toEqual(empty)
    expect(parseCockpitVideoLineup({})).toEqual(empty)
    expect(parseCockpitVideoLineup({ videos: 'nope' })).toEqual(empty)
  })
})
