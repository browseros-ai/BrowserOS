import { describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  DEFAULT_REPORT_MAX_TURNS,
  DEFAULT_REPORT_MODEL,
  generateEvalReport,
  runClaudeCodeReportAgent,
} from '../../scripts/generate-report'

async function writeRunFixture(): Promise<string> {
  const runDir = await mkdtemp(join(tmpdir(), 'eval-report-script-'))
  const taskDir = join(runDir, 'agisdk-networkin-10')
  await mkdir(join(taskDir, 'screenshots'), { recursive: true })
  await writeFile(
    join(runDir, 'summary.json'),
    JSON.stringify({
      total: 1,
      completed: 1,
      passRate: 0,
      avgDurationMs: 1234,
    }),
  )
  await writeFile(
    join(taskDir, 'metadata.json'),
    JSON.stringify({
      query_id: 'agisdk-networkin-10',
      dataset: 'agisdk-real',
      query: 'Send a follow-up message starting with "Following up on".',
      termination_reason: 'completed',
      total_duration_ms: 1234,
      total_steps: 2,
      screenshot_count: 1,
      final_answer: 'No app action was taken.',
      errors: [],
      warnings: [],
      agent_config: { type: 'single', model: 'kimi' },
      grader_results: {
        agisdk_state_diff: {
          score: 0,
          pass: false,
          reasoning: 'Some criteria failed',
          details: {
            per_criterion: [
              { passed: true, detail: 'message starts correctly' },
              { passed: false, detail: 'message was not sent' },
            ],
          },
        },
      },
    }),
  )
  await writeFile(
    join(taskDir, 'messages.jsonl'),
    [
      JSON.stringify({
        type: 'tool-input-available',
        timestamp: '2026-04-30T00:00:00.000Z',
        toolCallId: 'call-1',
        toolName: 'memory_search',
        input: { q: 'chat' },
      }),
      JSON.stringify({
        type: 'tool-output-error',
        timestamp: '2026-04-30T00:00:01.000Z',
        toolCallId: 'call-1',
        errorText: 'memory unavailable',
      }),
    ].join('\n'),
  )
  await writeFile(join(taskDir, 'screenshots', '1.png'), 'png')
  return runDir
}

describe('generate-report script', () => {
  it('delegates report.html creation to Claude Code', async () => {
    const runDir = await writeRunFixture()
    const outputPath = join(runDir, 'report.html')
    let prompt = ''

    await generateEvalReport({
      inputDir: runDir,
      outputPath,
      runAgent: async (invocation) => {
        prompt = invocation.prompt
        await writeFile(
          invocation.outputPath,
          '<!doctype html><h1>Claude-written report</h1>',
        )
      },
    })

    expect(await readFile(outputPath, 'utf-8')).toContain(
      'Claude-written report',
    )
    expect(prompt).toContain('AGI SDK Random-10 Failure Report')
    expect(prompt).toContain('summary.json')
    expect(prompt).toContain('messages.jsonl')
    expect(prompt).toContain('screenshots')
    expect(prompt).toContain('Deterministic run metrics')
    expect(prompt).toContain('"queryId": "agisdk-networkin-10"')
    expect(prompt).toContain('"toolCalls": 1')
    expect(prompt).toContain('"toolErrors": 1')
    expect(prompt).toContain('Duration by task')
    expect(prompt).toContain('Tool calls by task')
    expect(prompt).toContain(outputPath)
  })

  it('fails when the Claude Code agent does not write the report', async () => {
    const runDir = await writeRunFixture()

    await expect(
      generateEvalReport({
        inputDir: runDir,
        outputPath: join(runDir, 'missing-report.html'),
        runAgent: async () => {},
      }),
    ).rejects.toThrow('Report was not written')
  })

  it('runs Claude Code with Opus 4.6, full bypass, and bounded turns', async () => {
    const runDir = await writeRunFixture()
    const calls: unknown[] = []

    await runClaudeCodeReportAgent(
      {
        inputDir: runDir,
        outputPath: join(runDir, 'report.html'),
        prompt: 'write the report',
      },
      {
        query: async function* (call: unknown) {
          calls.push(call)
          yield { type: 'result', subtype: 'success', result: 'done' }
        },
        env: {
          CLAUDE_CODE_OAUTH_TOKEN: 'token',
          EVAL_R2_SECRET_ACCESS_KEY: 'secret',
          HOME: '/tmp/home',
          PATH: '/bin',
        },
      },
    )

    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({
      prompt: 'write the report',
      options: {
        cwd: runDir,
        model: DEFAULT_REPORT_MODEL,
        maxTurns: DEFAULT_REPORT_MAX_TURNS,
        permissionMode: 'bypassPermissions',
        allowDangerouslySkipPermissions: true,
      },
    })
    expect(JSON.stringify(calls[0])).not.toContain('secret')
  })
})
