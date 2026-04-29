#!/usr/bin/env bun

import { parseArgs } from 'node:util'
import { runEval } from './runner/eval-runner'

const { values } = parseArgs({
  args: Bun.argv.slice(2),
  options: {
    config: { type: 'string', short: 'c' },
    help: { type: 'boolean', short: 'h' },
  },
})

if (values.help) {
  console.log(`
BrowserOS Eval

Usage:
  bun run eval                          # Opens dashboard in config mode
  bun run eval --config <config.json>   # Runs eval with config file

Available agent types:
  - single                  Single LLM agent driven by the BrowserOS tool loop
  - orchestrator-executor   High-level planner + visual/text executor

Available graders:
  - performance_grader      Multi-axis grader using Claude Agent SDK
  - agisdk_state_diff       AGI SDK / REAL Bench state-diff grader
  - infinity_state          WebArena-Infinity verifier-script grader

Preset configs in configs/:
  - browseros-agent-weekly.json       Weekly eval (single agent)
  - browseros-oe-agent-weekly.json    Weekly eval (orchestrator + LLM executor)
  - browseros-oe-clado-weekly.json    Weekly eval (orchestrator + Clado executor)
  - agisdk-real-smoke.json            AGI SDK smoke run
  - infinity-hard-50.json             WebArena-Infinity hard-50 set
  - test-webvoyager.json              WebVoyager test
  - test-mind2web.json                Mind2Web test

Examples:
  bun run eval                                       # Dashboard config mode
  bun run eval -c configs/browseros-agent-weekly.json
  bun run eval -c configs/test-webvoyager.json
`)
  process.exit(0)
}

if (values.config) {
  try {
    await runEval({ configPath: values.config })
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
  process.exit(0)
} else {
  // No config — start dashboard in config mode, wait for user to configure and run
  const { startDashboard } = await import('./dashboard/server')
  startDashboard({
    tasks: [],
    configName: '',
    agentType: '',
    outputDir: '',
    configMode: true,
  })
  console.log(
    'Dashboard running at http://localhost:9900 — configure and run from the UI',
  )

  // Keep process alive until SIGINT
  await new Promise(() => {})
}
