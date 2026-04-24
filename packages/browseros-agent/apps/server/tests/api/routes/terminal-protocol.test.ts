/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { OPENCLAW_GATEWAY_CONTAINER_NAME } from '@browseros/shared/constants/openclaw'
import {
  parseTerminalClientMessage,
  serializeTerminalServerMessage,
} from '../../../src/api/services/terminal/terminal-protocol'
import {
  buildTerminalEnv,
  buildTerminalExecCommand,
  TERMINAL_HOME_DIR,
} from '../../../src/api/services/terminal/terminal-session'

describe('terminal protocol', () => {
  it('parses input messages', () => {
    expect(
      parseTerminalClientMessage('{"type":"input","data":"ls\\n"}'),
    ).toEqual({
      type: 'input',
      data: 'ls\n',
    })
  })

  it('parses resize messages', () => {
    expect(
      parseTerminalClientMessage('{"type":"resize","cols":120,"rows":40}'),
    ).toEqual({
      type: 'resize',
      cols: 120,
      rows: 40,
    })
  })

  it('returns null for malformed or invalid client messages', () => {
    expect(parseTerminalClientMessage('not-json')).toBeNull()
    expect(
      parseTerminalClientMessage('{"type":"resize","cols":0,"rows":40}'),
    ).toBeNull()
    expect(
      parseTerminalClientMessage(new Blob(['{"type":"input","data":"ls"}'])),
    ).toBeNull()
  })

  it('serializes server messages', () => {
    expect(
      serializeTerminalServerMessage({ type: 'output', data: 'hello' }),
    ).toBe('{"type":"output","data":"hello"}')
  })

  it('builds a limactl shell command rooted in the container home dir', () => {
    expect(
      buildTerminalExecCommand(
        'limactl',
        'browseros-vm',
        OPENCLAW_GATEWAY_CONTAINER_NAME,
        TERMINAL_HOME_DIR,
      ),
    ).toEqual([
      'limactl',
      'shell',
      'browseros-vm',
      '--',
      'nerdctl',
      'exec',
      '-it',
      '-w',
      '/home/node/.openclaw',
      OPENCLAW_GATEWAY_CONTAINER_NAME,
      '/bin/sh',
    ])
  })

  it('sets LIMA_HOME for terminal limactl sessions', () => {
    expect(buildTerminalEnv('/tmp/browseros-lima')).toEqual(
      expect.objectContaining({
        LIMA_HOME: '/tmp/browseros-lima',
        TERM: 'xterm-256color',
      }),
    )
  })
})
