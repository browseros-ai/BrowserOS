import type {
  JudgeAuditEnvelope,
  MonitoringFinalization,
  MonitoringSessionContext,
  MonitoringToolCallRecord,
} from './types'

export function buildJudgeAuditEnvelope(input: {
  context: MonitoringSessionContext
  toolCalls: MonitoringToolCallRecord[]
  finalization: MonitoringFinalization | null
}): JudgeAuditEnvelope {
  const envelope: JudgeAuditEnvelope = {
    run: input.context,
    toolCalls: input.toolCalls,
  }

  if (input.finalization) {
    envelope.finalization = input.finalization
  }

  return envelope
}
