# BrowserOS Analytics Events

All tracked events across the BrowserOS platform. Events are sent to PostHog.

## Event Naming Convention

- **Server events**: `browseros.server.<event>` — sent via `metrics.log()` from the MCP/HTTP server
- **Extension events**: `browseros.native.extension.<event>` — sent via `track()` from the Chrome extension
- **Native events**: `browseros.native.<event>` — sent from the Chromium browser process

## Server Events

Prefix: `browseros.server.`

| Event | Properties | Description |
|-------|-----------|-------------|
| `http_server.started` | `version` | Server boot completed |
| `mcp.request` | `scopeId` | Every `POST /mcp` from external MCP clients (Claude Code, Cursor, etc.) |
| `mcp.rejected` | — | MCP request rejected (e.g. auth failure) |
| `chat.request` | `provider`, `model` | Every `POST /chat` from the built-in BrowserOS agent |
| `chat.aborted` | — | Chat request was aborted by the user |
| `chat-v2.request` | — | Chat v2 endpoint request (deprecated) |
| `tool_executed` | `tool_name`, `duration_ms`, `success`, `error_message?`, `source` | A tool was executed. `source` = `mcp` (external client) or `chat` (built-in agent) |
| `rate_limit.triggered` | — | Rate limit was hit |

### Global Properties (attached to all server events)

| Property | Description |
|----------|-------------|
| `client_id` | Client identifier from BrowserOS config |
| `install_id` | Installation identifier |
| `browseros_version` | BrowserOS browser version |
| `chromium_version` | Underlying Chromium version |
| `server_version` | MCP server version |

## Extension Events — UI Interactions

Prefix: `browseros.native.extension.`

### Chat & Sidepanel

| Event | Properties | Description |
|-------|-----------|-------------|
| `ui.message.sent` | — | User sent a message |
| `ui.message.like` | — | User liked a message |
| `ui.message.dislike` | — | User disliked a message |
| `ui.provider.selected` | — | User selected an LLM provider |
| `ui.conversation.reset` | — | User reset conversation |
| `sidepanel.ai.triggered` | — | AI triggered from sidepanel |
| `sidepanel.mode.changed` | — | Chat/agent mode changed in sidepanel |
| `sidepanel.generation.stopped` | — | User stopped generation in sidepanel |
| `sidepanel.message.copied` | — | User copied a message in sidepanel |
| `sidepanel.suggestion.clicked` | — | User clicked a suggestion in sidepanel |
| `sidepanel.tab.toggled` | — | Tab toggled in sidepanel |
| `sidepanel.tab.removed` | — | Tab removed in sidepanel |
| `sidepanel.voice.recording_started` | — | Voice recording started in sidepanel |
| `sidepanel.voice.recording_stopped` | — | Voice recording stopped in sidepanel |
| `sidepanel.voice.transcription_completed` | — | Voice transcription completed in sidepanel |
| `sidepanel.voice.error` | — | Voice error in sidepanel |
| `glow.generation.stopped` | — | User stopped generation in glow mode |

### New Tab Page

| Event | Properties | Description |
|-------|-----------|-------------|
| `newtab.opened` | — | New tab page loaded |
| `newtab.ai.triggered` | `mode`, `tabs_count` | User triggered AI from new tab |
| `newtab.search.executed` | `search_engine` | User executed a search |
| `newtab.chat.started` | `mode`, `tabs_count` | Inline chat started on new tab |
| `newtab.chat.stopped` | — | Inline chat stopped |
| `newtab.chat.reset` | — | Inline chat reset |
| `newtab.chat.suggestion_clicked` | — | User clicked a chat suggestion |
| `newtab.chat.mode_changed` | — | Chat mode changed on new tab |
| `newtab.workspace.opened` | — | Workspace selector opened |
| `newtab.tabs.opened` | — | Tab picker opened |
| `newtab.tab.toggled` | `action` | Tab selected/deselected |
| `newtab.tab.removed` | — | Tab removed from context |
| `newtab.apps.opened` | `has_connected_apps`, `connected_count?` | Apps selector opened |
| `newtab.tip.dismissed` | — | Tip card dismissed |
| `newtab.voice.recording_started` | — | Voice recording started on new tab |
| `newtab.voice.recording_stopped` | — | Voice recording stopped on new tab |
| `newtab.voice.transcription_completed` | — | Voice transcription completed on new tab |
| `newtab.voice.error` | `error` | Voice error on new tab |
| `newtab.scheduled_task.viewed_results` | — | User viewed scheduled task results on new tab |
| `newtab.scheduled_task.view_more` | — | User clicked "view more" for scheduled tasks |

### Settings — AI Providers

| Event | Properties | Description |
|-------|-----------|-------------|
| `settings.page.viewed` | `page` | Settings page loaded (tracks which page) |
| `settings.ai_provider.added` | — | Custom AI provider added |
| `settings.hub_provider.added` | — | Hub provider added |
| `settings.search_provider.changed` | — | Search provider changed |
| `settings.mcp_promo_banner.clicked` | — | MCP promo banner clicked on providers page |

### Settings — OAuth Providers

| Event | Properties | Description |
|-------|-----------|-------------|
| `settings.chatgpt_pro.oauth_started` | — | ChatGPT Pro OAuth flow started |
| `settings.chatgpt_pro.oauth_completed` | — | ChatGPT Pro OAuth flow completed |
| `settings.chatgpt_pro.oauth_disconnected` | — | ChatGPT Pro disconnected |
| `settings.github_copilot.oauth_started` | — | GitHub Copilot OAuth flow started |
| `settings.github_copilot.oauth_completed` | — | GitHub Copilot OAuth flow completed |
| `settings.github_copilot.oauth_disconnected` | — | GitHub Copilot disconnected |
| `settings.qwen_code.oauth_started` | — | Qwen Code OAuth flow started |
| `settings.qwen_code.oauth_completed` | — | Qwen Code OAuth flow completed |
| `settings.qwen_code.oauth_disconnected` | — | Qwen Code disconnected |

### Settings — Kimi / Moonshot

| Event | Properties | Description |
|-------|-----------|-------------|
| `settings.kimi.api_key_configured` | — | Kimi API key was configured |
| `settings.kimi.api_key_guide_clicked` | — | User clicked Kimi API key guide |
| `ui.rate_limit.kimi_docs_clicked` | — | User clicked Kimi docs from rate limit notice |
| `ui.rate_limit.moonshot_platform_clicked` | — | User clicked Moonshot platform link from rate limit |

### Settings — MCP Server

| Event | Properties | Description |
|-------|-----------|-------------|
| `settings.mcp_external_access.enabled` | — | External MCP access enabled |
| `settings.mcp_external_access.disabled` | — | External MCP access disabled |
| `settings.mcp_server.restarted` | — | MCP server manually restarted |
| `settings.managed_mcp.added` | — | Managed MCP server connected (e.g. Gmail, Slack) |
| `settings.custom_mcp.added` | — | Custom MCP server added |

### Settings — Scheduled Tasks

| Event | Properties | Description |
|-------|-----------|-------------|
| `settings.scheduled_task.created` | — | New scheduled task created |
| `settings.scheduled_task.edited` | — | Scheduled task edited |
| `settings.scheduled_task.deleted` | — | Scheduled task deleted |
| `settings.scheduled_task.toggled` | — | Scheduled task enabled/disabled |
| `settings.scheduled_task.prompt_refined` | — | Task prompt was refined |
| `settings.scheduled_task.tested` | — | Scheduled task was tested |
| `settings.scheduled_task.viewed_results` | — | Task results viewed in settings |
| `settings.scheduled_task.cancelled` | — | Running task was cancelled |
| `settings.scheduled_task.retried` | — | Task run was retried |

### Onboarding

| Event | Properties | Description |
|-------|-----------|-------------|
| `onboarding.started` | — | Onboarding flow started |
| `onboarding.step.viewed` | — | Onboarding step viewed |
| `onboarding.step.completed` | — | Onboarding step completed |
| `onboarding.about.submitted` | — | User submitted "about me" info |
| `onboarding.soul.selected` | — | User selected a soul/persona |
| `onboarding.connect_apps.viewed` | — | Connect apps step viewed |
| `onboarding.app.connected` | — | App connected during onboarding |
| `onboarding.connect_apps.skipped` | — | Connect apps step skipped |
| `onboarding.signin.completed` | — | Sign-in completed during onboarding |
| `onboarding.signin.skipped` | — | Sign-in skipped during onboarding |
| `onboarding.demo.triggered` | — | Demo triggered during onboarding |
| `onboarding.feature.clicked` | — | Feature card clicked during onboarding |
| `onboarding.completed` | — | Onboarding flow completed |

### Breadcrumb Nudges

| Event | Properties | Description |
|-------|-----------|-------------|
| `breadcrumb.schedule.clicked` | — | Schedule nudge clicked |
| `breadcrumb.connect.clicked` | — | Connect app nudge clicked |
| `breadcrumb.connect.manual` | — | Manual connect triggered from nudge |
| `breadcrumb.connect.completed` | — | App connection completed from nudge |
| `breadcrumb.schedule.dismissed` | — | Schedule nudge dismissed |

### JTBD Popup

| Event | Properties | Description |
|-------|-----------|-------------|
| `ui.jtbd_popup.shown` | — | Jobs-to-be-done popup shown |
| `ui.jtbd_popup.clicked` | — | JTBD popup clicked |
| `ui.jtbd_popup.dismissed` | — | JTBD popup dismissed |

## Native Events (Chromium Browser Process)

Prefix: `browseros.native.`

These events come from the BrowserOS Chromium browser, not the extension or server.

| Event | Description |
|-------|-------------|
| `alive` | Heartbeat — BrowserOS instance is running |
| `llmhub.shown` | LLM Hub UI shown |
| `llmhub.panecount.changed` | Number of panes changed in LLM Hub |
| `llmhub.provider.switched` | Provider switched in LLM Hub |
| `llmchat.created` | New LLM chat session created |
| `llmchat.content.copied` | Content copied from LLM chat |
| `llmchat.provider.changed` | Provider changed in LLM chat |
| `llmchat.menu.hub` | Hub menu clicked |
| `llmchat.menu.help` | Help menu clicked |
| `llmchat.menu.newtab` | New tab menu clicked |
| `llmchat.menu.refresh` | Refresh menu clicked |
| `settings.provider.added` | Provider added via native settings |
| `settings.default_provider.changed` | Default provider changed via native settings |
| `server.ota.success` | Server OTA update succeeded |
| `server.ota.error` | Server OTA update failed |
| `server.ota.cleanup` | Server OTA cleanup ran |
| `server.ota.busy` | Server OTA busy (update in progress) |

## Other Events

| Event | Description |
|-------|-------------|
| `browseros.cdn.downloads` | CDN download tracking |
| `browseros.agent.feedback` | Agent feedback submitted |
| `browseros:update_ping` | Update ping (legacy) |
