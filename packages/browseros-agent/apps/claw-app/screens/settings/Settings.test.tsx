/**
 * Static-markup checks for the Settings screen. Stubs the settings hooks so
 * the test does not need a running backend.
 */

import { describe, expect, it, mock } from 'bun:test'
import type { AgentSettings } from '@browseros/claw-api'
import { renderToStaticMarkup } from 'react-dom/server'

let settings: { data?: AgentSettings; isError: boolean } = { isError: false }

mock.module('@/modules/api/agent-settings.hooks', () => ({
  useAgentSettings: () => settings,
  useUpdateAgentSettings: () => ({
    mutate: () => {},
    isPending: false,
    isError: false,
  }),
}))

const { Settings } = await import('./Settings')

// The switch's id lands on its hidden native checkbox.
function switchInput(html: string): string {
  return html.match(/<input[^>]*id="human-help-enabled"[^>]*>/)?.[0] ?? ''
}

describe('Settings screen', () => {
  it('shows the human-help switch on when the server has it on', () => {
    settings = { data: { humanHelpEnabled: true }, isError: false }
    const html = renderToStaticMarkup(<Settings />)
    expect(html).toContain('Ask for human help')
    expect(switchInput(html)).toContain('checked=""')
  })

  it('shows the human-help switch off when the server has it off', () => {
    settings = { data: { humanHelpEnabled: false }, isError: false }
    const html = renderToStaticMarkup(<Settings />)
    expect(switchInput(html)).not.toContain('checked')
  })

  it('disables the switch until settings load', () => {
    settings = { isError: false }
    expect(switchInput(renderToStaticMarkup(<Settings />))).toContain(
      'disabled=""',
    )
  })

  it('shows an error notice when settings cannot load', () => {
    settings = { isError: true }
    const html = renderToStaticMarkup(<Settings />)
    expect(html).toContain('Could not load settings')
    expect(html).not.toContain('human-help-enabled')
  })
})
