/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { promises as fs } from 'node:fs'
import { basename, dirname, join } from 'node:path'
import {
  applyEdits,
  findNodeAtLocation,
  getNodeValue,
  modify,
  type ParseError,
  parseTree,
} from 'jsonc-parser'
import type { LoggerInterface } from '../types/logger'

const FORMATTING = {
  formattingOptions: {
    insertSpaces: true,
    tabSize: 2,
  },
}

export interface EnsureClaudeCodeHttpTransportTagOptions {
  configPath: string
  serverName: string
  expectedUrl?: string
  expectedUrlPattern?: RegExp
  onlyIfMissing?: boolean
  logger?: LoggerInterface
}

export async function ensureClaudeCodeHttpTransportTag({
  configPath,
  serverName,
  expectedUrl,
  expectedUrlPattern,
  onlyIfMissing = false,
  logger,
}: EnsureClaudeCodeHttpTransportTagOptions): Promise<boolean> {
  try {
    const source = await readConfig(configPath)
    if (source === null) {
      logger?.debug('Claude Code MCP config missing; skipping transport tag', {
        configPath,
        serverName,
      })
      return false
    }

    const parseErrors: ParseError[] = []
    const tree = parseTree(source, parseErrors, { allowTrailingComma: true })
    if (!tree || parseErrors.length > 0) {
      logger?.warn('Claude Code MCP config is not valid JSON; skipped tag', {
        configPath,
        serverName,
      })
      return false
    }

    const entryNode = findNodeAtLocation(tree, ['mcpServers', serverName])
    if (entryNode?.type !== 'object') {
      logger?.debug('Claude Code MCP entry missing; skipping tag', {
        configPath,
        serverName,
      })
      return false
    }

    const typeNode = findNodeAtLocation(tree, [
      'mcpServers',
      serverName,
      'type',
    ])
    if (typeNode && (onlyIfMissing || getNodeValue(typeNode) === 'http')) {
      return false
    }

    if (expectedUrl !== undefined || expectedUrlPattern !== undefined) {
      const urlNode = findNodeAtLocation(tree, [
        'mcpServers',
        serverName,
        'url',
      ])
      const url = urlNode?.type === 'string' ? getNodeValue(urlNode) : null
      if (
        typeof url !== 'string' ||
        (expectedUrl !== undefined && url !== expectedUrl) ||
        (expectedUrlPattern !== undefined && !expectedUrlPattern.test(url))
      ) {
        logger?.debug(
          'Claude Code MCP entry URL mismatch; skipping transport tag',
          {
            configPath,
            serverName,
          },
        )
        return false
      }
    }

    const edits = modify(
      source,
      ['mcpServers', serverName, 'type'],
      'http',
      FORMATTING,
    )
    if (edits.length === 0) return false

    await atomicWrite(configPath, applyEdits(source, edits))
    return true
  } catch (err) {
    logger?.warn('Failed to ensure Claude Code MCP transport tag', {
      serverName,
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

async function readConfig(configPath: string): Promise<string | null> {
  try {
    return await fs.readFile(configPath, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
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
