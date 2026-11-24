#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Command line entry: copies passwords and signed-in sessions from a Firefox
 * profile into a running BrowserOS neo. Run it again at any time to bring
 * neo up to date; see README.md for what is and is not carried over.
 */

import { parseArgs } from 'node:util'
import {
  listFirefoxProfiles,
  pickDefaultProfile,
  resolveProfile,
} from './firefox/profiles'
import { runSync } from './sync'
import { defaultStatePath } from './sync-state'

const HELP = `Usage: bun run firefox-sync [options]

Copies Firefox passwords and signed-in sessions (cookies) into a running
BrowserOS neo. Safe to repeat: later runs update what changed in Firefox.

Options:
  --profile <name|dir|path>  Firefox profile (default: the most recently used install default)
  --list-profiles            List Firefox profiles and exit
  --only <cookies|passwords> Sync one kind only
  --domain <domain>          Only sites under this domain; repeat for more
  --missing-only             Keep cookies neo already has instead of replacing them
  --no-session-cookies       Skip session-only cookies from Firefox's session store
  --dry-run                  Show what would change; write nothing
  --cdp-port <port>          neo's CDP port (default: from neo's Local State, then 9110)
  --primary-password-stdin   Read the Firefox Primary Password from stdin
  --state <path>             Sync state file (default: ${defaultStatePath()})
  -h, --help                 Show this help
`

async function readStdin(): Promise<string> {
  const text = await new Response(Bun.stdin.stream()).text()
  return text.replace(/\r?\n$/, '')
}

async function main(): Promise<number> {
  const { values } = parseArgs({
    options: {
      profile: { type: 'string' },
      'list-profiles': { type: 'boolean' },
      only: { type: 'string' },
      domain: { type: 'string', multiple: true },
      'missing-only': { type: 'boolean' },
      'no-session-cookies': { type: 'boolean' },
      'dry-run': { type: 'boolean' },
      'cdp-port': { type: 'string' },
      'primary-password-stdin': { type: 'boolean' },
      state: { type: 'string' },
      help: { type: 'boolean', short: 'h' },
    },
  })
  if (values.help) {
    console.log(HELP)
    return 0
  }

  const profiles = listFirefoxProfiles()
  if (values['list-profiles']) {
    const chosen = pickDefaultProfile(profiles)
    for (const p of profiles) {
      const marks = [
        p === chosen ? 'default' : '',
        p.installDefault ? 'install-default' : '',
      ]
        .filter(Boolean)
        .join(', ')
      console.log(`${p.name}\t${p.path}${marks ? `\t(${marks})` : ''}`)
    }
    return 0
  }

  const profile = resolveProfile(values.profile, profiles)
  if (!profile) {
    console.error(
      values.profile
        ? `Firefox profile "${values.profile}" not found; see --list-profiles.`
        : 'No Firefox profile found.',
    )
    return 2
  }
  if (values.only && values.only !== 'cookies' && values.only !== 'passwords') {
    console.error('--only takes "cookies" or "passwords".')
    return 2
  }
  const cdpPort = values['cdp-port'] ? Number(values['cdp-port']) : undefined
  if (cdpPort !== undefined && !(Number.isInteger(cdpPort) && cdpPort > 0)) {
    console.error('--cdp-port takes a port number.')
    return 2
  }

  try {
    const ok = await runSync({
      profile,
      cookies: values.only !== 'passwords',
      sessionCookies: !values['no-session-cookies'],
      passwords: values.only !== 'cookies',
      domains: values.domain ?? [],
      missingOnly: values['missing-only'] === true,
      dryRun: values['dry-run'] === true,
      primaryPassword: values['primary-password-stdin']
        ? await readStdin()
        : undefined,
      cdpPort,
      statePath: values.state ?? defaultStatePath(),
      log: (line) => console.log(line),
    })
    return ok ? 0 : 1
  } catch (error) {
    // Messages name files and origins only; secrets never reach an Error.
    console.error(error instanceof Error ? error.message : String(error))
    return 2
  }
}

process.exit(await main())
