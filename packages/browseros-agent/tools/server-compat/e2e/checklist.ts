/** Release journeys that need the real extension; evidence records keep unexecuted work visible. */
export const checklist = [
  [
    'sidepanel-chat',
    'Send a real-key chat in the side panel; navigate, snapshot and screenshot tools render cards; persisted messages match.',
  ],
  [
    'panel-reconnect',
    'Reload the panel during streaming; /state and /stream resume the same run with no duplicated text/tool cards.',
  ],
  [
    'stop',
    'Stop mid-stream; UI stops, run becomes aborted, provider work ends and /status permits update.',
  ],
  [
    'newtab-chat',
    'Send chat from the new tab UI; context/selected text reach the server and history contains the turn.',
  ],
  [
    'chat-mode',
    'Enable chat mode; read-only catalog omits act and UI/server refuse browser mutations.',
  ],
  [
    'tab-ownership',
    'Agent-created tabs group under browseros/<task>; borrowed active tab has no ownership notice.',
  ],
  [
    'providers',
    'Add/test/edit/default/delete a provider in Settings; verify matching /providers rows and redaction.',
  ],
  [
    'oauth',
    'Start ChatGPT OAuth; 302 Location opens auth.openai.com and UI reflects the flow.',
  ],
  [
    'history',
    'List/open/delete conversation history; UI and /conversations agree after reload.',
  ],
  [
    'acp-turn',
    'Create/probe Claude and Codex agents; models list, MCP tool turn, image/text attachments and second turn work.',
  ],
  [
    'acp-edit',
    'Edit/delete an agent; old session is discarded; mode/effort and cancellation work after recreation.',
  ],
  [
    'scheduled-run',
    'Run now via real chrome.alarms; own background page, no nudge/group, visible result, run row and prune-to-15.',
  ],
  [
    'mcp-settings',
    'List tools, install into an isolated Claude Code config, uninstall; UI, file and manifest agree.',
  ],
  [
    'connector',
    'Klavis connect returns an auth URL; custom MCP tool roundtrip renders its result.',
  ],
  [
    'update-readiness',
    '/status.can_update is false during chat/MCP execution and true after completion/stop.',
  ],
  [
    'shutdown',
    'In the isolated supervised browser, POST /shutdown exits 0; record the actual Chromium supervisor behavior.',
  ],
  [
    'sigterm',
    'Signal only the isolated server; exit 3 and supervisor restart observed.',
  ],
  [
    'occupied-port',
    'Reserve an isolated port; server exits 2 and Chromium chooses another port.',
  ],
  [
    'browser-flows',
    'Synthetic two-origin page: snapshot/click/read/screenshot/download/PDF and scoped filesystem output readback.',
  ],
] as const
export type StepId = (typeof checklist)[number][0]
