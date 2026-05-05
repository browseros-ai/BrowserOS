import { describe, expect, it } from 'bun:test'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { adaptEvalConfigFile } from '../../src/suites/config-adapter'

describe('adaptEvalConfigFile', () => {
  it('preserves browseros-agent-weekly AGI SDK config semantics', async () => {
    const adapted = await adaptEvalConfigFile(
      'apps/eval/configs/legacy/browseros-agent-weekly.json',
    )

    expect(adapted.suite.id).toBe('browseros-agent-weekly')
    expect(adapted.suite.dataset).toBe('../../data/agisdk-real.jsonl')
    expect(adapted.suite.graders).toEqual(['agisdk_state_diff'])
    expect(adapted.suite.workers).toBe(3)
    expect(adapted.suite.restartBrowserPerTask).toBe(true)
    expect(adapted.suite.timeoutMs).toBe(1_800_000)
    expect(adapted.evalConfig.num_workers).toBe(3)
    expect(adapted.evalConfig.browseros.server_url).toBe(
      'http://127.0.0.1:9110',
    )
  })

  it('keeps API key env names public while omitting secret values', async () => {
    const adapted = await adaptEvalConfigFile(
      'apps/eval/configs/legacy/browseros-agent-weekly.json',
      {
        env: { OPENROUTER_API_KEY: 'secret-openrouter-value' },
      },
    )

    expect(adapted.variant.publicMetadata.agent.apiKeyEnv).toBe(
      'OPENROUTER_API_KEY',
    )
    expect(JSON.stringify(adapted.variant.publicMetadata)).not.toContain(
      'secret-openrouter-value',
    )
  })

  it('adapts BrowserOS AGI SDK comparison configs', async () => {
    const kimi = await adaptEvalConfigFile(
      'apps/eval/configs/legacy/browseros-agent-kimi-k2-5-agisdk-real.json',
    )
    const opus = await adaptEvalConfigFile(
      'apps/eval/configs/legacy/browseros-agent-opus-4-6-agisdk-real.json',
    )

    expect(kimi.suite.id).toBe('browseros-agent-kimi-k2-5-agisdk-real')
    expect(kimi.evalConfig.agent).toMatchObject({
      type: 'single',
      provider: 'openai-compatible',
      model: 'moonshotai/kimi-k2.5',
    })
    expect(kimi.evalConfig.num_workers).toBe(3)

    expect(opus.suite.id).toBe('browseros-agent-opus-4-6-agisdk-real')
    expect(opus.evalConfig.agent).toMatchObject({
      type: 'single',
      provider: 'bedrock',
      model: 'global.anthropic.claude-opus-4-6-v1',
      region: 'AWS_REGION',
      accessKeyId: 'AWS_ACCESS_KEY_ID',
      secretAccessKey: 'AWS_SECRET_ACCESS_KEY',
    })
    expect(opus.evalConfig.num_workers).toBe(2)
  })

  it('adapts claude-code configs without provider credentials', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'claude-code-config-'))
    const configPath = join(dir, 'claude-code-agisdk.json')
    await writeFile(
      configPath,
      JSON.stringify({
        agent: {
          type: 'claude-code',
          model: 'opus',
        },
        dataset: 'tasks.jsonl',
        num_workers: 1,
        restart_server_per_task: false,
        browseros: {
          server_url: 'http://127.0.0.1:9110',
          headless: false,
        },
      }),
    )

    const adapted = await adaptEvalConfigFile(configPath, { env: {} })

    expect(adapted.suite.agent).toEqual({ type: 'claude-code' })
    expect(adapted.variant.agent).toMatchObject({
      provider: 'claude-code',
      model: 'opus',
    })
  })
})
