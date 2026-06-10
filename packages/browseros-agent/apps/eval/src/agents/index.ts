import { ClaudeCodeEvaluator } from './claude-code'
import { OrchestratorExecutorEvaluator } from './orchestrator-executor'
import { SingleAgentEvaluator } from './single-agent'
import type { AgentContext, AgentEvaluator } from './types'

export function createAgent(context: AgentContext): AgentEvaluator {
  switch (context.config.agent.type) {
    case 'single':
      return new SingleAgentEvaluator(context)
    case 'orchestrator-executor':
      return new OrchestratorExecutorEvaluator(context)
    case 'claude-code':
      return new ClaudeCodeEvaluator(context)
  }
}

export type { AgentContext, AgentEvaluator } from './types'
