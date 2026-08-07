import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { StatusBadge } from './StatusBadge'

describe('StatusBadge', () => {
  it('renders cancelled sessions as stopped', () => {
    expect(renderToStaticMarkup(<StatusBadge status="cancelled" />)).toContain(
      'Stopped',
    )
  })
})
