import { type ToolSet, tool } from 'ai'
import { z } from 'zod'
import { getConnectorCatalog } from '../api/services/klavis'

const appNames = getConnectorCatalog()
  .map((server) => server.name)
  .join(', ')

// UI-control sentinel tools: the JSON text they return is intercepted by the chat UI to render
// interactive cards (schedule / app-connection). They are not browser actions, so they live with
// the agent rather than on the MCP browser surface.
function sentinel(payload: Record<string, unknown>) {
  const text = JSON.stringify(payload)
  return {
    content: [{ type: 'text' as const, text }],
    isError: false,
  }
}

export function buildNudgeToolSet(): ToolSet {
  return {
    suggest_schedule: tool({
      description:
        'Call this to suggest scheduling a task. Use in two cases: (1) MANDATORY after completing a task that could run on a recurring schedule (news, monitoring, reports, price tracking, data gathering). (2) Immediately when the user explicitly asks to schedule, automate, or repeat the current task - do NOT ask for clarification, infer all parameters from context. Do NOT call if the task requires real-time user interaction.',
      inputSchema: z.object({
        query: z.string().describe('The original user query to schedule'),
        suggestedName: z
          .string()
          .describe(
            'A short, descriptive name for the scheduled task (e.g. "Morning News Briefing")',
          ),
        scheduleType: z
          .enum(['daily', 'hourly'])
          .describe('How often the task should run'),
        scheduleTime: z
          .string()
          .optional()
          .describe(
            'Suggested time for daily tasks in HH:MM format (e.g. "09:00").',
          ),
      }),
      execute: async (args) =>
        sentinel({
          type: 'schedule_suggestion',
          query: args.query,
          suggestedName: args.suggestedName,
          scheduleType: args.scheduleType,
          scheduleTime: args.scheduleTime ?? '09:00',
        }),
      toModelOutput: ({ output }) => ({
        type: 'text',
        value:
          (output as { content: { text?: string }[] }).content[0]?.text ?? '',
      }),
    }),
    suggest_app_connection: tool({
      description: `BLOCKING DECISION - Call when the user's request relates to a Connect Apps service but you don't have MCP tools for it. Your response must contain ONLY this tool call with zero text. The appName must be one of: ${appNames}.`,
      inputSchema: z.object({
        appName: z
          .string()
          .describe(
            'The name of the app to connect (must match a supported app name exactly)',
          ),
        reason: z
          .string()
          .describe(
            'A brief, user-friendly explanation of why connecting this app would help',
          ),
      }),
      execute: async (args) =>
        sentinel({
          type: 'app_connection',
          appName: args.appName,
          reason: args.reason,
        }),
      toModelOutput: ({ output }) => ({
        type: 'text',
        value:
          (output as { content: { text?: string }[] }).content[0]?.text ?? '',
      }),
    }),
  }
}
