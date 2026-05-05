#!/usr/bin/env bun

import { mkdir, stat } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { query as claudeQuery } from '@anthropic-ai/claude-agent-sdk'
import { readRunMetricSummary } from '../src/reporting/task-metrics'

export const DEFAULT_REPORT_MODEL = 'claude-opus-4-6'
export const DEFAULT_REPORT_MAX_TURNS = 300

type Env = Record<string, string | undefined>
type ClaudeQuery = (input: unknown) => AsyncIterable<Record<string, unknown>>

export interface ReportAgentInvocation {
  inputDir: string
  outputPath: string
  prompt: string
}

export interface GenerateEvalReportOptions {
  inputDir: string
  outputPath: string
  runAgent?: (invocation: ReportAgentInvocation) => Promise<void>
}

interface ClaudeReportAgentDeps {
  query?: ClaudeQuery
  env?: Env
}

function usage(): string {
  return `Usage: bun scripts/generate-report.ts --input <run-dir> --output <report.html>`
}

function parseArgs(
  argv: string[],
): Pick<GenerateEvalReportOptions, 'inputDir' | 'outputPath'> {
  let inputDir = ''
  let outputPath = ''
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (arg === '--input' || arg === '--run') {
      inputDir = argv[++i] ?? ''
    } else if (arg === '--output' || arg === '--out') {
      outputPath = argv[++i] ?? ''
    } else if (arg === '--help' || arg === '-h') {
      console.log(usage())
      process.exit(0)
    }
  }
  if (!inputDir || !outputPath) {
    throw new Error(usage())
  }
  return { inputDir, outputPath }
}

function claudeCodeEnv(env: Env): Env {
  return {
    CLAUDE_CODE_OAUTH_TOKEN: env.CLAUDE_CODE_OAUTH_TOKEN,
    ANTHROPIC_API_KEY: env.ANTHROPIC_API_KEY,
    HOME: env.HOME,
    PATH: env.PATH,
    SHELL: env.SHELL,
    TMPDIR: env.TMPDIR,
    TMP: env.TMP,
    TEMP: env.TEMP,
    USER: env.USER,
    CLAUDECODE: '',
  }
}

async function buildReportPrompt(
  inputDir: string,
  outputPath: string,
): Promise<string> {
  const metrics = await readRunMetricSummary(inputDir)

  return `Analyze this BrowserOS eval run and write a shareable HTML report.

Run directory: ${inputDir}
Output file to write: ${outputPath}

You are running with the run directory as cwd. Inspect the local artifacts:
- summary.json for run totals and pass rate
- each task directory's metadata.json for query, final answer, timing, screenshots, and grader results
- each task directory's messages.jsonl for tool calls, tool errors, and recent trajectory
- screenshots/ for visual evidence
- grader-artifacts/ when present for grader-specific context

Write the final report directly to the output file path above. Do not print the
report instead of writing it. Do not modify any input artifacts. The only file
you should create or overwrite is the requested report.html.

The report should follow the style and density of the Shadowfax AGI SDK report:
- Title like "AGI SDK Random-10 Failure Report" or a run-specific equivalent
- Run directory and note that screenshots are embedded as data URIs
- Summary cards for total tasks, passed, failed, pass rate, average duration, average steps, and average tool calls
- A Metrics section with compact charts for Duration by task, Steps by task, Tool calls by task, and Tool errors by task
- Task Summary table with task id, status, score, duration, steps, and prompt
- Include tool calls and tool errors in the Task Summary table
- Failure sections with stable anchors using each task id, for example <section id="agisdk-networkin-10">
- For each failed task: Diagnosis, Evidence, Next Check, final screenshot, AGI SDK / grader criteria, final answer, and recent trajectory events
- Make failure links in the summary table point to the task anchors
- Keep the HTML self-contained: inline CSS and embedded final screenshots as data:image/png;base64 URIs
- Escape user/model text correctly so task outputs cannot break the page

Analysis guidance:
- Focus on why the model failed: task understanding, browser/tool usage, missing verification, tool errors, max-step/timeout, bad final answer, or grader ambiguity
- Use messages.jsonl strategically. Do not paste huge DOM outputs into the report. Summarize only the relevant recent trajectory and evidence.
- Limit trajectory analysis to the most relevant 200-300 events/calls across the run. Prefer failed tasks and the final/key actions for each failure.
- If a grader criterion is boolean-only or ambiguous, say so and identify what additional artifact would make it debuggable.

Deterministic run metrics computed from metadata.json and messages.jsonl:
\`\`\`json
${JSON.stringify(metrics, null, 2)}
\`\`\`

After writing the file, verify that ${outputPath} exists and is non-empty.`
}

async function assertRunDir(inputDir: string): Promise<void> {
  const inputStat = await stat(inputDir).catch(() => null)
  if (!inputStat?.isDirectory()) {
    throw new Error(`Not a run directory: ${inputDir}`)
  }
}

async function assertReportWritten(outputPath: string): Promise<void> {
  const outputStat = await stat(outputPath).catch(() => null)
  if (!outputStat?.isFile() || outputStat.size === 0) {
    throw new Error(`Report was not written: ${outputPath}`)
  }
}

export async function runClaudeCodeReportAgent(
  invocation: ReportAgentInvocation,
  deps: ClaudeReportAgentDeps = {},
): Promise<void> {
  const query = deps.query ?? (claudeQuery as unknown as ClaudeQuery)
  let resultSubtype: string | undefined

  for await (const message of query({
    prompt: invocation.prompt,
    options: {
      cwd: invocation.inputDir,
      model: DEFAULT_REPORT_MODEL,
      systemPrompt:
        'You are an eval failure analyst. Produce a concise, evidence-backed, self-contained HTML report from local run artifacts.',
      permissionMode: 'bypassPermissions',
      allowDangerouslySkipPermissions: true,
      maxTurns: DEFAULT_REPORT_MAX_TURNS,
      env: claudeCodeEnv(deps.env ?? process.env),
    },
  })) {
    if (message.type === 'result') {
      resultSubtype =
        typeof message.subtype === 'string' ? message.subtype : undefined
    }
  }

  if (resultSubtype && resultSubtype !== 'success') {
    throw new Error(`Claude Code report agent failed: ${resultSubtype}`)
  }
}

export async function generateEvalReport(
  options: GenerateEvalReportOptions,
): Promise<void> {
  const inputDir = resolve(options.inputDir)
  const outputPath = resolve(options.outputPath)

  await assertRunDir(inputDir)
  await mkdir(dirname(outputPath), { recursive: true })

  const invocation = {
    inputDir,
    outputPath,
    prompt: await buildReportPrompt(inputDir, outputPath),
  }
  await (options.runAgent ?? runClaudeCodeReportAgent)(invocation)
  await assertReportWritten(outputPath)
}

if (import.meta.main) {
  try {
    await generateEvalReport(parseArgs(Bun.argv.slice(2)))
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
}
