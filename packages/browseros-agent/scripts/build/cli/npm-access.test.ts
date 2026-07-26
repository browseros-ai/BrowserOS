import { describe, expect, test } from 'bun:test'

import { verifyNpmPublishAccess } from './npm-access'

describe('verifyNpmPublishAccess', () => {
  test('accepts an authenticated package owner', () => {
    const calls: string[][] = []
    const result = verifyNpmPublishAccess('browseros-cli', (args) => {
      calls.push(args)
      if (args[0] === 'whoami') {
        return 'browseros_eng\n'
      }
      if (args.join(' ') === 'owner ls browseros-cli') {
        return 'browseros_eng <eng@felafax.ai>\nother_owner <owner@example.com>\n'
      }
      if (
        args.join(' ') ===
        'access list collaborators browseros-cli browseros_eng --json'
      ) {
        return JSON.stringify({ browseros_eng: 'read-write' })
      }
      throw new Error(`unexpected npm args: ${args.join(' ')}`)
    })

    expect(result).toEqual({
      packageName: 'browseros-cli',
      user: 'browseros_eng',
      owners: ['browseros_eng', 'other_owner'],
      access: 'read-write',
    })
    expect(calls).toEqual([
      ['whoami'],
      ['owner', 'ls', 'browseros-cli'],
      [
        'access',
        'list',
        'collaborators',
        'browseros-cli',
        'browseros_eng',
        '--json',
      ],
    ])
  })

  test('rejects a token user that is not a package owner', () => {
    expect(() =>
      verifyNpmPublishAccess('browseros-cli', (args) => {
        if (args[0] === 'whoami') {
          return 'ci_bot\n'
        }
        if (args.join(' ') === 'owner ls browseros-cli') {
          return 'browseros_eng <eng@felafax.ai>\n'
        }
        throw new Error(`unexpected npm args: ${args.join(' ')}`)
      }),
    ).toThrow(
      'NPM_TOKEN authenticates as ci_bot, but browseros-cli owners are: browseros_eng',
    )
  })

  test('rejects an unreadable owner list', () => {
    expect(() =>
      verifyNpmPublishAccess('browseros-cli', (args) => {
        if (args[0] === 'whoami') {
          return 'browseros_eng\n'
        }
        if (args.join(' ') === 'owner ls browseros-cli') {
          return '\n'
        }
        throw new Error(`unexpected npm args: ${args.join(' ')}`)
      }),
    ).toThrow('No npm owners found for browseros-cli')
  })

  test('rejects owner tokens without read-write package access', () => {
    expect(() =>
      verifyNpmPublishAccess('browseros-cli', (args) => {
        if (args[0] === 'whoami') {
          return 'browseros_eng\n'
        }
        if (args.join(' ') === 'owner ls browseros-cli') {
          return 'browseros_eng <eng@felafax.ai>\n'
        }
        if (
          args.join(' ') ===
          'access list collaborators browseros-cli browseros_eng --json'
        ) {
          return JSON.stringify({ browseros_eng: 'read-only' })
        }
        throw new Error(`unexpected npm args: ${args.join(' ')}`)
      }),
    ).toThrow('does not have read-write access to browseros-cli')
  })
})
