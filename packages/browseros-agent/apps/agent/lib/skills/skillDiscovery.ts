import { executionHistoryStorage } from '../execution-history/storage'

export interface SuggestedSkill {
  name: string
  description: string
  content: string
}

const DISCOVERY_PROMPT = `You are a BrowserOS Workflow Expert. Analyze the following list of recent tasks performed by the user. 
Identify repetitive patterns or common browser-based activities that could be automated by a "Skill" (a set of instructions for the agent).

Suggested skills should have:
1. A clear, short name.
2. A description of when it's useful.
3. Detailed Markdown instructions for the agent.

Return the suggestions as a JSON array: [{ "name": "...", "description": "...", "content": "..." }]

Recent Tasks:
{{TASKS}}`

/**
 * Safkan Skill Discovery Logic
 * Analyzes local execution history to suggest automations.
 */
export async function discoverSkills(sendMessage: (text: string) => Promise<string>): Promise<SuggestedSkill[]> {
  const history = await executionHistoryStorage.getValue()
  if (!history || Object.keys(history).length === 0) return []

  // Extract recent tasks (last 15 tasks)
  const allTasks = Object.values(history)
    .flatMap((h) => h.tasks)
    .sort((a, b) => b.timestamp - a.timestamp)
    .slice(0, 15)

  if (allTasks.length < 3) return []

  const taskList = allTasks
    .map((t) => `- ${t.promptText} (Tools: ${t.toolCalls?.map((tc) => tc.name).join(', ') || 'none'})`)
    .join('\n')

  const prompt = DISCOVERY_PROMPT.replace('{{TASKS}}', taskList)

  try {
    // This expects the UI to handle the LLM interaction through the provided sendMessage function
    // or a dedicated discovery endpoint.
    const response = await sendMessage(prompt)
    const jsonMatch = response.match(/\[.*\]/s)
    if (jsonMatch) {
      return JSON.parse(jsonMatch[0]) as SuggestedSkill[]
    }
  } catch (error) {
    console.error('Skill discovery failed:', error)
  }

  return []
}
