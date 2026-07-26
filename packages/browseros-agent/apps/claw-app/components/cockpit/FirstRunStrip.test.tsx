/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { FirstRunStrip } from './FirstRunStrip'

describe('FirstRunStrip', () => {
  it('renders each step with its number and title', () => {
    const html = renderToStaticMarkup(
      <FirstRunStrip
        steps={[
          {
            number: '01',
            title: 'Install BrowserClaw as an MCP.',
            status: 'active',
          },
          { number: '02', title: 'Prompt your agent.', status: 'upcoming' },
          { number: '03', title: 'Watch it here.', status: 'upcoming' },
        ]}
      />,
    )
    expect(html).toContain('01')
    expect(html).toContain('02')
    expect(html).toContain('03')
    expect(html).toContain('Install BrowserClaw as an MCP.')
    expect(html).toContain('Prompt your agent.')
    expect(html).toContain('Watch it here.')
  })

  it('done status paints green check styling; active status paints accent styling', () => {
    const html = renderToStaticMarkup(
      <FirstRunStrip
        steps={[
          { number: '01', title: 'Done step.', status: 'done' },
          { number: '02', title: 'Active step.', status: 'active' },
          { number: '03', title: 'Upcoming step.', status: 'upcoming' },
        ]}
      />,
    )
    // Green pip on done, accent pip on active, dashed border on upcoming.
    expect(html).toContain('text-green')
    expect(html).toContain('bg-accent')
    expect(html).toContain('border-dashed')
  })
})
