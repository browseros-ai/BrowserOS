import { describe, expect, it } from 'bun:test'
import {
  describeMcpChange,
  describeModeChange,
  describeWorkspaceChange,
} from '../../../src/api/services/chat-service.helpers'

describe('describeMcpChange', () => {
  it('reports a newly connected integration', () => {
    expect(describeMcpChange('klavis:ready', 'slack,klavis:ready')).toBe(
      'The following app integrations were connected: slack. Their tools are now available.',
    )
  })

  it('reports a disconnected integration', () => {
    expect(describeMcpChange('slack,klavis:ready', 'klavis:ready')).toBe(
      'The following app integrations were disconnected: slack. Their tools are no longer available.',
    )
  })

  it('reports both additions and removals in one notice', () => {
    expect(describeMcpChange('slack,klavis:ready', 'notion,klavis:ready')).toBe(
      'The following app integrations were disconnected: slack. Their tools are no longer available. The following app integrations were connected: notion. Their tools are now available.',
    )
  })

  it('reports Klavis becoming ready when the connected set is unchanged', () => {
    expect(
      describeMcpChange('slack,klavis:connecting', 'slack,klavis:ready'),
    ).toBe(
      'Klavis app integration tools are now available for the following connected apps: slack.',
    )
  })

  it('falls back to a generic notice when nothing meaningful changed', () => {
    expect(describeMcpChange('klavis:ready', 'klavis:stopped')).toBe(
      'Connected app integrations changed during this conversation. Use only tools that are currently registered.',
    )
  })

  it('treats an undefined previous key as no prior integrations', () => {
    expect(describeMcpChange(undefined, 'slack,klavis:ready')).toBe(
      'The following app integrations were connected: slack. Their tools are now available.',
    )
  })
})

describe('describeWorkspaceChange', () => {
  it('describes a workspace disconnect', () => {
    const notice = describeWorkspaceChange('/ws', undefined, false)
    expect(notice).toContain('The user disconnected the workspace')
    expect(notice).toContain('are no longer available')
  })

  it('describes a workspace connect in agent mode', () => {
    expect(describeWorkspaceChange(undefined, '/ws', false)).toBe(
      'The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: /ws',
    )
  })

  it('describes a workspace connect in read-only chat mode', () => {
    expect(describeWorkspaceChange(undefined, '/ws', true)).toBe(
      'The user connected a workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools. filesystem_read can only read BrowserOS-generated output files returned in this session.',
    )
  })

  it('describes a workspace switch in agent mode', () => {
    expect(describeWorkspaceChange('/old', '/new', false)).toBe(
      'The user switched workspace during this conversation. Filesystem tools now use the new working directory: /new',
    )
  })

  it('describes a workspace switch in read-only chat mode', () => {
    expect(describeWorkspaceChange('/old', '/new', true)).toBe(
      'The user switched workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools. filesystem_read can only read BrowserOS-generated output files returned in this session.',
    )
  })
})

describe('describeModeChange', () => {
  it('describes switching to chat mode with a workspace connected', () => {
    expect(describeModeChange(true, true)).toContain(
      'workspace filesystem tools other than reading BrowserOS output files are no longer available',
    )
  })

  it('describes switching to chat mode without a workspace', () => {
    expect(describeModeChange(true, false)).toBe(
      'The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them or modify files.',
    )
  })

  it('describes switching to agent mode with a workspace connected', () => {
    expect(describeModeChange(false, true)).toContain(
      'the workspace filesystem tools are available again',
    )
  })

  it('describes switching to agent mode without a workspace', () => {
    expect(describeModeChange(false, false)).toBe(
      'The user switched to agent mode during this conversation. You can interact with pages and perform browser actions again.',
    )
  })
})
