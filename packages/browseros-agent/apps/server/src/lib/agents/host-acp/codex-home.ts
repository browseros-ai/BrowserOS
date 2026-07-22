/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Builds a CODEX_HOME overlay for the acpx chat path that disables Codex's
 * bundled in-app browser plugin (`browser@openai-bundled`), so a browser
 * task is driven through the BrowserOS MCP tools instead of Codex's own
 * in-app browser. The overlay symlinks every real `~/.codex` entry except
 * `config.toml`, which is regenerated with the plugin disabled. Two
 * failsafes keep it resilient: the symlink farm re-syncs to `~/.codex` on
 * every call (new files picked up, removed ones pruned), and the config
 * edit is committed only if a real TOML round-trip proves it changed
 * exactly the one field and nothing else.
 */

import { randomUUID } from 'node:crypto'
import {
  lstat,
  mkdir,
  readdir,
  readFile,
  readlink,
  rename,
  rm,
  symlink,
  unlink,
  writeFile,
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { basename, dirname, join } from 'node:path'
import { logger } from '../../logger'

export const IN_APP_BROWSER_PLUGIN_KEY = 'browser@openai-bundled'

const PLUGIN_TABLE_HEADER = `[plugins."${IN_APP_BROWSER_PLUGIN_KEY}"]`
const CONFIG_FILE = 'config.toml'

/**
 * Pure textual transform: return `configToml` with the in-app browser
 * plugin disabled, touching only its `[plugins."browser@openai-bundled"]`
 * table and leaving everything else byte-for-byte. Idempotent.
 */
export function disableInAppBrowserPlugin(configToml: string): string {
  const lines = configToml.split('\n')
  const headerIndex = lines.findIndex(
    (line) => line.trim() === PLUGIN_TABLE_HEADER,
  )

  if (headerIndex === -1) {
    const prefix =
      configToml.length === 0 || configToml.endsWith('\n') ? '' : '\n'
    return `${configToml}${prefix}\n${PLUGIN_TABLE_HEADER}\nenabled = false\n`
  }

  // Scan the table body until the next table/array header or EOF.
  for (let i = headerIndex + 1; i < lines.length; i++) {
    const trimmed = lines[i]?.trim() ?? ''
    if (trimmed.startsWith('[')) break
    if (/^enabled\s*=/.test(trimmed)) {
      lines[i] = 'enabled = false'
      return lines.join('\n')
    }
  }

  // Table present but no `enabled` key: insert one right after the header.
  lines.splice(headerIndex + 1, 0, 'enabled = false')
  return lines.join('\n')
}

/**
 * True when `editedText` is exactly `sourceText` with only the in-app
 * browser plugin's `enabled` set to `false`. Uses a real TOML round-trip
 * so a config-format change can never slip a corrupt overlay through.
 * Returns false (never throws) when either side fails to parse.
 */
export function verifyBrowserDisabled(
  sourceText: string,
  editedText: string,
): boolean {
  let srcObj: Record<string, unknown>
  let outObj: Record<string, unknown>
  try {
    srcObj = Bun.TOML.parse(sourceText) as Record<string, unknown>
    outObj = Bun.TOML.parse(editedText) as Record<string, unknown>
  } catch {
    return false
  }

  const expected = structuredClone(srcObj)
  if (!expected.plugins) expected.plugins = {}
  const plugins = expected.plugins as Record<string, Record<string, unknown>>
  plugins[IN_APP_BROWSER_PLUGIN_KEY] = {
    ...(plugins[IN_APP_BROWSER_PLUGIN_KEY] ?? {}),
    enabled: false,
  }

  const outPlugins = outObj.plugins as
    | Record<string, Record<string, unknown>>
    | undefined
  return (
    outPlugins?.[IN_APP_BROWSER_PLUGIN_KEY]?.enabled === false &&
    deepEqual(outObj, expected)
  )
}

/**
 * Materialize (or refresh) the CODEX_HOME overlay with the in-app browser
 * disabled. Returns the overlay path, or null on any failure so the caller
 * falls back to the real `~/.codex`. Never throws; never writes a partial
 * or corrupt config.
 */
export async function materializeCodexBrowserlessHome(input: {
  browserosDir: string
  sourceCodexHome?: string
}): Promise<string | null> {
  try {
    const source =
      input.sourceCodexHome?.trim() ||
      process.env.CODEX_HOME?.trim() ||
      join(homedir(), '.codex')
    const target = join(input.browserosDir, 'acpx-state', 'codex-home-no-iab')
    await mkdir(target, { recursive: true })

    if (!(await syncSymlinkFarm(source, target))) {
      logger.error(
        'Incomplete Codex home overlay (a required symlink failed); ' +
          'using real home so Codex keeps its auth and settings',
      )
      return null
    }
    const wrote = await writeBrowserlessConfig(source, target)
    return wrote ? target : null
  } catch (error) {
    logger.warn(
      'Failed to materialize browserless Codex home; using real home',
      {
        error: error instanceof Error ? error.message : String(error),
      },
    )
    return null
  }
}

/**
 * Failsafe 1: converge the overlay's symlinks to mirror `source`. Returns
 * false when any required link could not be created, so the caller falls
 * back to the real home rather than handing Codex an incomplete overlay.
 */
async function syncSymlinkFarm(
  source: string,
  target: string,
): Promise<boolean> {
  const sourceEntries = await readdir(source)
  const keep = new Set<string>()
  let allOk = true
  for (const name of sourceEntries) {
    if (name === CONFIG_FILE) continue
    keep.add(name)
    if (!(await ensureSymlink(join(source, name), join(target, name)))) {
      allOk = false
    }
  }

  let targetEntries: string[]
  try {
    targetEntries = await readdir(target)
  } catch {
    return allOk
  }
  for (const name of targetEntries) {
    if (name === CONFIG_FILE || keep.has(name)) continue
    const linkPath = join(target, name)
    if (await isSymlink(linkPath)) {
      await unlink(linkPath).catch(() => undefined)
    }
  }
  return allOk
}

/**
 * Ensure `linkPath` is a symlink to `sourcePath`. Returns true on success,
 * including the benign case where a concurrent session created the same
 * link first (EEXIST already pointing at the right target). Returns false
 * on a genuine failure.
 */
async function ensureSymlink(
  sourcePath: string,
  linkPath: string,
): Promise<boolean> {
  try {
    const info = await lstat(linkPath).catch(() => null)
    if (info?.isSymbolicLink()) {
      const current = await readlink(linkPath).catch(() => null)
      if (current === sourcePath) return true
      await unlink(linkPath)
    } else if (info) {
      await rm(linkPath, { recursive: true, force: true })
    }
    await symlink(sourcePath, linkPath)
    return true
  } catch (error) {
    // A concurrent session may have created the same link between our
    // lstat and symlink; that is not a failure if it points where we want.
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      const current = await readlink(linkPath).catch(() => null)
      if (current === sourcePath) return true
    }
    logger.warn('Failed to create a Codex home overlay symlink', {
      linkPath,
      error: error instanceof Error ? error.message : String(error),
    })
    return false
  }
}

/** Failsafe 2: write the overlay config only if the edit round-trips clean. */
async function writeBrowserlessConfig(
  source: string,
  target: string,
): Promise<boolean> {
  // A missing config.toml is a valid empty config; any other read error
  // (permissions, I/O) must propagate so the caller falls back to the real
  // home instead of writing an overlay that drops the user's settings.
  let sourceText: string
  try {
    sourceText = await readFile(join(source, CONFIG_FILE), 'utf8')
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    sourceText = ''
  }
  const editedText = disableInAppBrowserPlugin(sourceText)
  if (!verifyBrowserDisabled(sourceText, editedText)) {
    logger.error(
      'Codex config.toml did not round-trip after disabling the in-app ' +
        'browser plugin; leaving CODEX_HOME unset to avoid a corrupt overlay',
    )
    return false
  }
  await writeFileAtomic(join(target, CONFIG_FILE), editedText)
  return true
}

async function writeFileAtomic(path: string, content: string): Promise<void> {
  const temporaryPath = join(
    dirname(path),
    `.${basename(path)}.${process.pid}.${randomUUID()}.tmp`,
  )
  try {
    await writeFile(temporaryPath, content, 'utf8')
    await rename(temporaryPath, path)
  } catch (error) {
    await rm(temporaryPath, { force: true }).catch(() => undefined)
    throw error
  }
}

async function isSymlink(path: string): Promise<boolean> {
  try {
    return (await lstat(path)).isSymbolicLink()
  } catch {
    return false
  }
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (typeof a !== typeof b) return false
  if (a === null || b === null) return a === b
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) {
      return false
    }
    return a.every((item, index) => deepEqual(item, b[index]))
  }
  if (typeof a === 'object' && typeof b === 'object') {
    const aObj = a as Record<string, unknown>
    const bObj = b as Record<string, unknown>
    const aKeys = Object.keys(aObj)
    const bKeys = Object.keys(bObj)
    if (aKeys.length !== bKeys.length) return false
    return aKeys.every((key) => key in bObj && deepEqual(aObj[key], bObj[key]))
  }
  return false
}
