import { z } from 'zod'
import { OAUTH_MCP_SERVERS } from '../lib/clients/klavis/oauth-mcp-servers'
import { defineToolWithCategory } from './framework'

const appNames = OAUTH_MCP_SERVERS.map((s) => s.name).join(', ')
const defineAssistantTool = defineToolWithCategory('assistant')

export const suggest_schedule = defineAssistantTool({
  name: 'suggest_schedule',
  description:
    'Call this to suggest scheduling a task. Use in two cases: (1) MANDATORY after completing a task that could run on a recurring schedule (news, monitoring, reports, price tracking, data gathering). (2) Immediately when the user explicitly asks to schedule, automate, or repeat the current task — do NOT ask for clarification, infer all parameters from context. Do NOT call if the task requires real-time user interaction.',
  input: z.object({
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
        'Suggested time for daily tasks in HH:MM format (e.g. "09:00"). Ignored for hourly.',
      ),
  }),
  handler: async (args, _ctx, response) => {
    response.text(
      JSON.stringify({
        type: 'schedule_suggestion',
        query: args.query,
        suggestedName: args.suggestedName,
        scheduleType: args.scheduleType,
        scheduleTime: args.scheduleTime ?? '09:00',
      }),
    )
  },
})

export const suggest_app_connection = defineAssistantTool({
  name: 'suggest_app_connection',
  description: `BLOCKING DECISION — Call after tab grouping but before any browser work when the user's request relates to a Connect Apps service but you don't have MCP tools for it. Your response must contain ONLY this tool call with zero text. The appName must be one of: ${appNames}.`,
  input: z.object({
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
  handler: async (args, _ctx, response) => {
    response.text(
      JSON.stringify({
        type: 'app_connection',
        appName: args.appName,
        reason: args.reason,
      }),
    )
  },
})
