export interface AgentListItem {
  key: string
  agentId: string
  name: string
  source: 'agent-harness'
  runtimeLabel: string
  modelLabel: string
  detail: string
  canChat: boolean
  canDelete: boolean
}
