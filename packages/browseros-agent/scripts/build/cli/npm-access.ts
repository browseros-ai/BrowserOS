#!/usr/bin/env bun

import { execFileSync } from 'node:child_process'

export interface NpmPublishAccess {
  packageName: string
  user: string
  owners: string[]
  access: string
}

type NpmRunner = (args: string[]) => string

/** Confirms the configured npm token belongs to a package owner before release side effects. */
export function verifyNpmPublishAccess(
  packageName: string,
  runNpm: NpmRunner = runNpmCommand,
): NpmPublishAccess {
  const normalizedPackageName = packageName.trim()
  if (!normalizedPackageName) {
    throw new Error('Missing npm package name')
  }

  const user = runWhoami(runNpm)
  const owners = parseNpmOwners(runNpm(['owner', 'ls', normalizedPackageName]))

  if (owners.length === 0) {
    throw new Error(`No npm owners found for ${normalizedPackageName}`)
  }

  if (!owners.includes(user)) {
    throw new Error(
      `NPM_TOKEN authenticates as ${user}, but ${normalizedPackageName} owners are: ${owners.join(', ')}. Use a token for a listed owner or add ${user} as an owner before releasing.`,
    )
  }

  const access = readCollaboratorAccess(normalizedPackageName, user, runNpm)

  return { packageName: normalizedPackageName, user, owners, access }
}

function parseNpmOwners(output: string): string[] {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => line.split(/\s+/)[0])
    .filter(Boolean)
}

function runWhoami(runNpm: NpmRunner): string {
  let user: string
  try {
    user = runNpm(['whoami']).trim()
  } catch (error) {
    throw new Error(
      `NPM_TOKEN could not authenticate with npm: ${errorMessage(error)}`,
    )
  }
  if (!user) {
    throw new Error('NPM_TOKEN authenticated with npm but returned no username')
  }
  return user
}

function readCollaboratorAccess(
  packageName: string,
  user: string,
  runNpm: NpmRunner,
): string {
  let output: string
  try {
    output = runNpm([
      'access',
      'list',
      'collaborators',
      packageName,
      user,
      '--json',
    ])
  } catch (error) {
    throw new Error(
      `NPM_TOKEN could not read collaborator access for ${packageName}: ${errorMessage(error)}`,
    )
  }

  const access = parseCollaboratorAccess(output, user)
  if (access !== 'read-write') {
    throw new Error(
      `NPM_TOKEN authenticates as ${user}, but ${user} does not have read-write access to ${packageName}`,
    )
  }
  return access
}

function parseCollaboratorAccess(output: string, user: string): string {
  let parsed: unknown
  try {
    parsed = JSON.parse(output)
  } catch {
    throw new Error(`Could not parse npm collaborator access for ${user}`)
  }

  if (!parsed || typeof parsed !== 'object') {
    throw new Error(`No npm collaborator access found for ${user}`)
  }

  const collaborators = parsed as Record<string, unknown>
  const access = collaborators[user] ?? collaborators[user.toLowerCase()]
  return typeof access === 'string' ? access : ''
}

function runNpmCommand(args: string[]): string {
  return execFileSync('npm', args, {
    encoding: 'utf-8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim()
}

function parseArgs(args: string[]): { packageName: string } {
  let packageName = 'browseros-cli'

  for (let index = 0; index < args.length; index += 1) {
    const raw = args[index]
    if (raw !== '--package') {
      throw new Error(`Unexpected argument ${raw}`)
    }
    const value = args[index + 1]
    if (!value || value.startsWith('--')) {
      throw new Error('Missing value for --package')
    }
    packageName = value
    index += 1
  }

  return { packageName }
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    const stderr = 'stderr' in error ? String(error.stderr).trim() : ''
    return stderr || error.message
  }
  return String(error)
}

function main(): void {
  const { packageName } = parseArgs(process.argv.slice(2))
  const access = verifyNpmPublishAccess(packageName)
  console.log(
    `Validated npm publish access for ${access.packageName}: ${access.user}`,
  )
}

if (import.meta.main) {
  try {
    main()
  } catch (error) {
    console.error(`::error::${errorMessage(error)}`)
    process.exit(1)
  }
}
