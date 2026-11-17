#!/usr/bin/env bun
/** Records the launcher's browser ownership facts before replacing this process with Chromium. */

import { constants } from 'node:fs'
import { access, mkdir, writeFile } from 'node:fs/promises'
import { isAbsolute, join } from 'node:path'
import type { BrowserRecord } from './model'

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

function absolute(value: string, name: string): string {
  if (!isAbsolute(value)) throw new Error(`${name} must be absolute`)
  return value
}

/** CI extras may add browser features, but cannot replace the launcher's private profile or CDP. */
export function browserRecord(
  launcherArgs: string[],
  env: NodeJS.ProcessEnv,
): BrowserRecord {
  const profiles = launcherArgs.flatMap((arg, index) =>
    arg.startsWith('--user-data-dir=')
      ? [arg.slice('--user-data-dir='.length)]
      : arg === '--user-data-dir'
        ? [launcherArgs[index + 1] ?? '']
        : [],
  )
  const [profile] = profiles
  if (!profile || profiles.length !== 1)
    throw new Error('Expected exactly one launcher-owned --user-data-dir')
  const scale = required(env, 'SDK_DESKTOP_FIXTURE_SCALE')
  if (scale !== '1' && scale !== '2')
    throw new Error('SDK_DESKTOP_FIXTURE_SCALE must be 1 or 2')
  const extras = (env.BROWSEROS_TEST_EXTRA_ARGS ?? '')
    .split(' ')
    .filter(Boolean)
  if (
    extras.some((arg) =>
      /^--(?:user-data-dir|remote-debugging-port|force-device-scale-factor)(?:=|$)/.test(
        arg,
      ),
    ) ||
    launcherArgs.some((arg) => arg.startsWith('--force-device-scale-factor'))
  )
    throw new Error(
      'Browser extras cannot replace launcher ownership or native density',
    )
  return {
    profile: absolute(profile, 'browser profile'),
    home: absolute(required(env, 'HOME'), 'browser HOME'),
    downloads: absolute(
      required(env, 'SDK_DESKTOP_FIXTURE_DOWNLOADS'),
      'browser downloads',
    ),
    args: [
      ...launcherArgs,
      ...extras,
      ...(scale === '2' ? ['--force-device-scale-factor=2'] : []),
    ],
    scale: scale === '2' ? 2 : 1,
  }
}

/** The download preference must exist before Chromium starts; HOME alone does not select it. */
export async function prepareBrowser(
  launcherArgs: string[],
  env: NodeJS.ProcessEnv,
): Promise<{ binary: string; browser: BrowserRecord }> {
  const binary = absolute(
    required(env, 'SDK_DESKTOP_FIXTURE_BROWSER'),
    'browser binary',
  )
  const recordPath = absolute(
    required(env, 'SDK_DESKTOP_FIXTURE_BROWSER_RECORD'),
    'browser record',
  )
  await access(binary, constants.X_OK)
  const browser = browserRecord(launcherArgs, env)
  await mkdir(browser.downloads, { recursive: true, mode: 0o700 })
  await mkdir(join(browser.profile, 'Default'), {
    recursive: true,
    mode: 0o700,
  })
  await writeFile(
    join(browser.profile, 'Default/Preferences'),
    JSON.stringify({
      download: {
        default_directory: browser.downloads,
        prompt_for_download: false,
      },
    }),
    { flag: 'wx', mode: 0o600 },
  )
  await writeFile(recordPath, `${JSON.stringify(browser, null, 2)}\n`, {
    flag: 'wx',
    mode: 0o600,
  })
  return { binary, browser }
}

if (import.meta.main) {
  try {
    const { binary, browser } = await prepareBrowser(
      process.argv.slice(2),
      process.env,
    )
    if (!process.execve)
      throw new Error('This Bun version does not support process.execve')
    // Replacing the adapter preserves the detached browser group and its profile argv marker.
    // Spawning another lifetime owner here would undermine test:browser's proven cleanup.
    process.execve(binary, [binary, ...browser.args], process.env)
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 125
  }
}
