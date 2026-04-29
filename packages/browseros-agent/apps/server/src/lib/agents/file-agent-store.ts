/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { getBrowserosDir } from '../browseros-dir'
import { logger } from '../logger'
import {
  resolveDefaultModelId,
  resolveDefaultReasoningEffort,
} from './agent-catalog'
import type { AgentAdapter, AgentDefinition } from './agent-types'

interface AgentStoreFile {
  version: 1
  agents: AgentDefinition[]
}

export interface CreateAgentInput {
  name: string
  adapter: AgentAdapter
  modelId?: string
  reasoningEffort?: string
}

export class FileAgentStore {
  private readonly filePath: string
  private writeQueue: Promise<unknown> = Promise.resolve()

  constructor(options: { filePath?: string } = {}) {
    this.filePath =
      options.filePath ??
      join(getBrowserosDir(), 'agents', 'harness', 'agents.json')
  }

  async list(): Promise<AgentDefinition[]> {
    const file = await this.read()
    const agents = [...file.agents].sort((a, b) => b.updatedAt - a.updatedAt)
    logger.debug('Agent harness store listed agents', {
      count: agents.length,
      filePath: this.filePath,
    })
    return agents
  }

  async get(id: string): Promise<AgentDefinition | null> {
    const file = await this.read()
    const agent = file.agents.find((entry) => entry.id === id) ?? null
    logger.debug('Agent harness store loaded agent', {
      agentId: id,
      found: Boolean(agent),
      adapter: agent?.adapter,
      filePath: this.filePath,
    })
    return agent
  }

  async create(input: CreateAgentInput): Promise<AgentDefinition> {
    return this.withWriteLock(async () => {
      const now = Date.now()
      const id = randomUUID()
      const agent: AgentDefinition = {
        id,
        name: input.name.trim(),
        adapter: input.adapter,
        modelId: input.modelId ?? resolveDefaultModelId(input.adapter),
        reasoningEffort:
          input.reasoningEffort ?? resolveDefaultReasoningEffort(input.adapter),
        permissionMode: 'approve-all',
        sessionKey: `agent:${id}:main`,
        createdAt: now,
        updatedAt: now,
      }
      const file = await this.read()
      await this.write({ ...file, agents: [...file.agents, agent] })
      logger.info('Agent harness store created agent', {
        agentId: agent.id,
        name: agent.name,
        adapter: agent.adapter,
        modelId: agent.modelId,
        reasoningEffort: agent.reasoningEffort,
        sessionKey: agent.sessionKey,
        filePath: this.filePath,
      })
      return agent
    })
  }

  async delete(id: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      const file = await this.read()
      const agents = file.agents.filter((agent) => agent.id !== id)
      if (agents.length === file.agents.length) return false
      await this.write({ ...file, agents })
      logger.info('Agent harness store deleted agent', {
        agentId: id,
        filePath: this.filePath,
      })
      return true
    })
  }

  private async read(): Promise<AgentStoreFile> {
    try {
      const raw = await readFile(this.filePath, 'utf8')
      const parsed = JSON.parse(raw) as AgentStoreFile
      if (parsed.version !== 1 || !Array.isArray(parsed.agents)) {
        return emptyStoreFile()
      }
      return parsed
    } catch (err) {
      if (isNotFoundError(err)) return emptyStoreFile()
      throw err
    }
  }

  private async write(file: AgentStoreFile): Promise<void> {
    await mkdir(dirname(this.filePath), { recursive: true })
    const tmpPath = `${this.filePath}.${process.pid}.${Date.now()}.tmp`
    await writeFile(tmpPath, `${JSON.stringify(file, null, 2)}\n`, 'utf8')
    await rename(tmpPath, this.filePath)
  }

  private withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    const result = this.writeQueue.then(fn, fn)
    this.writeQueue = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }
}

function emptyStoreFile(): AgentStoreFile {
  return { version: 1, agents: [] }
}

function isNotFoundError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'ENOENT'
  )
}
