/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { promises as fs } from 'node:fs'
import { basename, dirname, join } from 'node:path'
import type { LoggerInterface } from '@browseros/shared/types/logger'
import TOML from '@iarna/toml'
import {
  type AgentId,
  type AgentScope,
  ForeignEntryError,
  type McpManager,
  resolveAgentMcpConfigPath,
  resolveAgentSurface,
  ServerNotFoundError,
} from 'agent-mcp-manager'
import {
  applyEdits,
  findNodeAtLocation,
  getNodeValue,
  modify,
  type ParseError,
  parseTree,
} from 'jsonc-parser'
import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'
import type { Harness } from '../routes/agents/schemas'
import { LEGACY_BROWSEROS_MCP_SERVER_NAMES } from '../shared/mcp-url-common'
import type { ConnectionState } from './browseros-connect'

type LegacyBrowserosServerName =
  (typeof LEGACY_BROWSEROS_MCP_SERVER_NAMES)[number]
type JsonParentKey = 'mcpServers' | 'servers' | 'context_servers'

interface SweepLegacyBrowserosEntriesOptions {
  mgr?: McpManager
  logger?: LoggerInterface
}

interface DetectLegacyBrowserosEntriesOptions {
  logger?: LoggerInterface
}

export interface HealLegacyBrowserosEntriesOptions {
  harnessToAgentId?: Partial<Record<Harness, AgentId | null>>
  resolveConfigPath?: (agent: AgentId, scope?: AgentScope) => Promise<string>
  connect?: (harness: Harness) => Promise<ConnectionState>
  logger?: LoggerInterface
}

export type LegacyBrowserosHealSummary = Partial<
  Record<Harness, LegacyBrowserosServerName[]>
>

const FORMATTING = {
  formattingOptions: {
    insertSpaces: true,
    tabSize: 2,
  },
}

// Mirrors agent-mcp-manager 0.0.3's _vendor/catalog.ts parent keys.
// Re-check this map when the library catalog changes.
const JSON_PARENT_KEY_BY_AGENT: Partial<Record<AgentId, JsonParentKey>> = {
  'claude-code': 'mcpServers',
  'claude-desktop': 'mcpServers',
  cursor: 'mcpServers',
  vscode: 'servers',
  gemini: 'mcpServers',
  zed: 'context_servers',
}

export async function detectLegacyBrowserosEntries(
  agent: AgentId,
  configPath: string,
  options: DetectLegacyBrowserosEntriesOptions = {},
): Promise<LegacyBrowserosServerName[]> {
  if (agent === 'codex') {
    return detectTomlLegacyEntries(agent, configPath, options)
  }
  return detectJsonLegacyEntries(agent, configPath, options)
}

export async function sweepLegacyBrowserosEntries(
  agent: AgentId,
  configPath: string,
  options: SweepLegacyBrowserosEntriesOptions = {},
): Promise<LegacyBrowserosServerName[]> {
  const log = options.logger ?? logger
  const mgr = options.mgr ?? getMcpManager()
  const safeNames = await detectLegacyBrowserosEntries(agent, configPath, {
    logger: log,
  })
  if (safeNames.length === 0) return []

  const removed = new Set<LegacyBrowserosServerName>()
  for (const serverName of safeNames) {
    try {
      const result = await mgr.unlink({ serverName, agent, configPath })
      if (result.removed) removed.add(serverName)
      await removeManifestServerIfUnlinked(mgr, serverName, log)
    } catch (err) {
      if (
        err instanceof ForeignEntryError ||
        err instanceof ServerNotFoundError
      ) {
        continue
      }
      log.warn('legacy BrowserOS MCP manifest cleanup failed', {
        agent,
        serverName,
        configPath,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  const diskRemoved =
    agent === 'codex'
      ? await sweepTomlLegacyEntries(agent, configPath, safeNames, log)
      : await sweepJsonLegacyEntries(agent, configPath, safeNames, log)
  for (const name of diskRemoved) removed.add(name)

  return safeNames.filter((name) => removed.has(name))
}

export async function healLegacyBrowserosEntries(
  options: HealLegacyBrowserosEntriesOptions = {},
): Promise<LegacyBrowserosHealSummary> {
  const log = options.logger ?? logger
  const harnessToAgentId =
    options.harnessToAgentId ??
    (await import('./harness-install')).HARNESS_TO_AGENT_ID
  const resolveConfigPath =
    options.resolveConfigPath ??
    ((agent: AgentId, scope: AgentScope = 'system') =>
      resolveAgentMcpConfigPath(agent, scope))
  const connect =
    options.connect ??
    (async (harness: Harness) =>
      (await import('./browseros-connect')).connectBrowserosToHarness(harness))
  const healed: LegacyBrowserosHealSummary = {}

  for (const [harness, agent] of Object.entries(harnessToAgentId) as Array<
    [Harness, AgentId | null | undefined]
  >) {
    if (!agent) continue
    let configPath: string
    try {
      configPath = await resolveConfigPath(agent, 'system')
    } catch (err) {
      log.debug('legacy BrowserOS MCP heal skipped unresolved config path', {
        harness,
        agent,
        error: err instanceof Error ? err.message : String(err),
      })
      continue
    }
    if (!(await pathExists(configPath))) continue

    const legacyNames = await detectLegacyBrowserosEntries(agent, configPath, {
      logger: log,
    })
    if (legacyNames.length === 0) continue

    const result = await connect(harness)
    if (!result.installed) {
      log.warn('legacy BrowserOS MCP heal connect failed', {
        harness,
        agent,
        configPath,
        message: result.message,
      })
      continue
    }
    healed[harness] = legacyNames
  }

  return healed
}

async function removeManifestServerIfUnlinked(
  mgr: McpManager,
  serverName: LegacyBrowserosServerName,
  log: LoggerInterface,
): Promise<void> {
  try {
    const links = await mgr.listLinks({ serverNames: [serverName] })
    if (links.length === 0) {
      await mgr.remove({ serverName, unlinkFirst: false })
    }
  } catch (err) {
    if (err instanceof ServerNotFoundError) return
    log.warn('legacy BrowserOS MCP manifest server removal failed', {
      serverName,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

async function detectJsonLegacyEntries(
  agent: AgentId,
  configPath: string,
  options: DetectLegacyBrowserosEntriesOptions,
): Promise<LegacyBrowserosServerName[]> {
  const parentKey = JSON_PARENT_KEY_BY_AGENT[agent]
  if (!parentKey) return []
  const parsed = await readJsonConfig(configPath, options.logger ?? logger)
  if (!parsed) return []

  const safeNames: LegacyBrowserosServerName[] = []
  for (const name of LEGACY_BROWSEROS_MCP_SERVER_NAMES) {
    if (!canSweepNameForAgent(name, agent)) continue
    const entryNode = findNodeAtLocation(parsed.tree, [parentKey, name])
    if (!entryNode) continue
    if (isBrowserosLocalMcpEntry(getNodeValue(entryNode))) {
      safeNames.push(name)
    } else {
      warnSkippedForeignEntry(options.logger ?? logger, agent, configPath, name)
    }
  }
  return safeNames
}

async function sweepJsonLegacyEntries(
  agent: AgentId,
  configPath: string,
  names: readonly LegacyBrowserosServerName[],
  log: LoggerInterface,
): Promise<LegacyBrowserosServerName[]> {
  const parentKey = JSON_PARENT_KEY_BY_AGENT[agent]
  if (!parentKey) return []
  const parsed = await readJsonConfig(configPath, log)
  if (!parsed) return []

  let source = parsed.source
  const removed: LegacyBrowserosServerName[] = []
  for (const name of names) {
    const current = parseJsonConfigSource(source, configPath, log)
    if (!current) break
    const entryNode = findNodeAtLocation(current.tree, [parentKey, name])
    if (!entryNode) continue
    if (!isBrowserosLocalMcpEntry(getNodeValue(entryNode))) {
      warnSkippedForeignEntry(log, agent, configPath, name)
      continue
    }
    const edits = modify(source, [parentKey, name], undefined, FORMATTING)
    if (edits.length === 0) continue
    source = applyEdits(source, edits)
    removed.push(name)
  }

  if (removed.length > 0 && source !== parsed.source) {
    await atomicWrite(configPath, source)
  }
  return removed
}

async function detectTomlLegacyEntries(
  agent: AgentId,
  configPath: string,
  options: DetectLegacyBrowserosEntriesOptions,
): Promise<LegacyBrowserosServerName[]> {
  const parsed = await readTomlConfig(configPath, options.logger ?? logger)
  if (!parsed) return []
  const servers = mcpServersTable(parsed.doc)
  if (!servers) return []

  const safeNames: LegacyBrowserosServerName[] = []
  for (const name of LEGACY_BROWSEROS_MCP_SERVER_NAMES) {
    if (!canSweepNameForAgent(name, agent)) continue
    if (!Object.hasOwn(servers, name)) continue
    if (isBrowserosLocalMcpEntry(servers[name])) {
      safeNames.push(name)
    } else {
      warnSkippedForeignEntry(options.logger ?? logger, agent, configPath, name)
    }
  }
  return safeNames
}

async function sweepTomlLegacyEntries(
  agent: AgentId,
  configPath: string,
  names: readonly LegacyBrowserosServerName[],
  log: LoggerInterface,
): Promise<LegacyBrowserosServerName[]> {
  const parsed = await readTomlConfig(configPath, log)
  if (!parsed) return []
  const servers = mcpServersTable(parsed.doc)
  if (!servers) return []

  const removed: LegacyBrowserosServerName[] = []
  for (const name of names) {
    if (!Object.hasOwn(servers, name)) continue
    if (!isBrowserosLocalMcpEntry(servers[name])) {
      warnSkippedForeignEntry(log, agent, configPath, name)
      continue
    }
    delete servers[name]
    removed.push(name)
  }

  if (removed.length > 0) {
    const next = TOML.stringify(
      parsed.doc as Parameters<typeof TOML.stringify>[0],
    )
    if (next !== parsed.source) await atomicWrite(configPath, next)
  }
  return removed
}

function parseJsonConfigSource(
  source: string,
  configPath: string,
  log: LoggerInterface,
): { source: string; tree: NonNullable<ReturnType<typeof parseTree>> } | null {
  const parseErrors: ParseError[] = []
  const tree = parseTree(source, parseErrors, { allowTrailingComma: true })
  if (!tree || parseErrors.length > 0) {
    log.warn('MCP JSON config is not valid JSON; skipped legacy sweep', {
      configPath,
    })
    return null
  }
  return { source, tree }
}

async function readJsonConfig(
  configPath: string,
  log: LoggerInterface,
): Promise<{
  source: string
  tree: NonNullable<ReturnType<typeof parseTree>>
} | null> {
  const source = await readConfig(configPath)
  if (source === null) return null
  if (!source.trim()) return null
  return parseJsonConfigSource(source, configPath, log)
}

async function readTomlConfig(
  configPath: string,
  log: LoggerInterface,
): Promise<{ source: string; doc: Record<string, unknown> } | null> {
  const source = await readConfig(configPath)
  if (source === null) return null
  if (!source.trim()) return null
  try {
    const doc = TOML.parse(source)
    if (!doc || typeof doc !== 'object') return null
    return { source, doc: doc as Record<string, unknown> }
  } catch {
    log.warn('MCP TOML config is not valid TOML; skipped legacy sweep', {
      configPath,
    })
    return null
  }
}

function mcpServersTable(
  doc: Record<string, unknown>,
): Record<string, unknown> | null {
  const table = doc.mcp_servers
  if (!table || typeof table !== 'object' || Array.isArray(table)) return null
  return table as Record<string, unknown>
}

function canSweepNameForAgent(
  name: LegacyBrowserosServerName,
  agent: AgentId,
): boolean {
  if (name !== 'browseros-stdio') return true
  return resolveAgentSurface(agent, 'system').supportedTransports.includes(
    'http',
  )
}

function isBrowserosLocalMcpEntry(entry: unknown): boolean {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return false
  const record = entry as Record<string, unknown>
  if (typeof record.url === 'string') return isBrowserosLocalMcpUrl(record.url)
  if (record.command !== 'npx') return false
  if (!Array.isArray(record.args)) return false
  const args = record.args.every((arg) => typeof arg === 'string')
    ? (record.args as string[])
    : []
  const normalizedArgs = args[0] === '-y' ? args.slice(1) : args
  return (
    normalizedArgs.length === 2 &&
    normalizedArgs[0] === 'mcp-remote' &&
    isBrowserosLocalMcpUrl(normalizedArgs[1])
  )
}

function isBrowserosLocalMcpUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return (
      url.protocol === 'http:' &&
      (url.hostname === '127.0.0.1' || url.hostname === 'localhost') &&
      url.pathname === '/mcp' &&
      url.search === '' &&
      url.hash === ''
    )
  } catch {
    return false
  }
}

function warnSkippedForeignEntry(
  log: LoggerInterface,
  agent: AgentId,
  configPath: string,
  serverName: LegacyBrowserosServerName,
): void {
  log.warn('legacy BrowserOS MCP entry did not match safe sweep shape', {
    agent,
    configPath,
    serverName,
  })
}

async function readConfig(configPath: string): Promise<string | null> {
  try {
    return await fs.readFile(configPath, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await fs.stat(path)
    return true
  } catch {
    return false
  }
}

async function atomicWrite(
  configPath: string,
  contents: string,
): Promise<void> {
  const dir = dirname(configPath)
  const tmp = join(
    dir,
    `.${basename(configPath)}.tmp-${process.pid}-${Date.now()}-${Math.random()
      .toString(16)
      .slice(2)}`,
  )
  try {
    await fs.writeFile(tmp, contents, 'utf8')
    await fs.rename(tmp, configPath)
  } catch (err) {
    await fs.unlink(tmp).catch(() => {})
    throw err
  }
}
