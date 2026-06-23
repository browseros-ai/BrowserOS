/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ConnectorCatalogItem } from './types'

const CONNECTOR_CATALOG: ConnectorCatalogItem[] = [
  { name: 'Gmail', description: 'Send, read, and search emails' },
  { name: 'Google Calendar', description: 'Create events, manage calendars' },
  { name: 'Google Docs', description: 'Create and edit documents' },
  { name: 'Google Drive', description: 'Upload, download, and manage files' },
  { name: 'Google Sheets', description: 'Create and edit spreadsheets' },
  { name: 'Slack', description: 'Post messages, manage channels' },
  { name: 'LinkedIn', description: 'Post updates, manage connections' },
  { name: 'Notion', description: 'Create pages, manage databases' },
  { name: 'Airtable', description: 'Manage bases, tables, and records' },
  { name: 'Confluence', description: 'Create and manage documentation' },
  { name: 'GitHub', description: 'Manage repos, issues, pull requests' },
  { name: 'GitLab', description: 'Manage repos, issues, merge requests' },
  { name: 'Linear', description: 'Create issues, manage cycles' },
  { name: 'Jira', description: 'Create issues, manage sprints' },
  { name: 'Figma', description: 'Access and manage design files' },
  { name: 'Salesforce', description: 'Manage leads, contacts, opportunities' },
  { name: 'ClickUp', description: 'Manage tasks, projects, and workflows' },
  { name: 'Asana', description: 'Organize and track team projects' },
  { name: 'Monday', description: 'Manage work and team collaboration' },
  { name: 'Microsoft Teams', description: 'Chat, meet, and collaborate' },
  { name: 'Outlook Mail', description: 'Send, read, and manage emails' },
  { name: 'Outlook Calendar', description: 'Schedule meetings and events' },
  { name: 'Supabase', description: 'Manage databases and backend services' },
  { name: 'Vercel', description: 'Deploy and manage web applications' },
  { name: 'Postman', description: 'Test and manage APIs' },
  { name: 'Stripe', description: 'Manage payments and subscriptions' },
  { name: 'Cloudflare', description: 'Manage domains, DNS, and security' },
  { name: 'Brave Search', description: 'Search the web privately' },
  { name: 'Mem0', description: 'Store and retrieve AI memory' },
  { name: 'Dropbox', description: 'Store and share files in the cloud' },
  { name: 'OneDrive', description: 'Store and sync files with Microsoft' },
  { name: 'WordPress', description: 'Manage websites and blog content' },
  { name: 'YouTube', description: 'Access video info and transcripts' },
  { name: 'Box', description: 'Manage and share enterprise files' },
  { name: 'HubSpot', description: 'Manage contacts, deals, and marketing' },
  { name: 'PostHog', description: 'Query analytics, manage feature flags' },
  { name: 'Mixpanel', description: 'Analyze user behavior and metrics' },
  { name: 'Discord', description: 'Send messages and manage servers' },
  { name: 'WhatsApp', description: 'Send messages and manage conversations' },
  { name: 'Shopify', description: 'Manage products, orders, and store' },
  { name: 'Cal.com', description: 'Schedule meetings and manage availability' },
  { name: 'Resend', description: 'Send transactional and marketing emails' },
  { name: 'Google Forms', description: 'Create and manage forms and surveys' },
  { name: 'Zendesk', description: 'Manage support tickets and customers' },
  { name: 'Intercom', description: 'Manage customer messaging and support' },
]

/** Returns the curated Klavis connector catalog in API response order. */
export function getConnectorCatalog(): ConnectorCatalogItem[] {
  return CONNECTOR_CATALOG.map((server) => ({ ...server }))
}

export function getConnectorServerNames(): string[] {
  return CONNECTOR_CATALOG.map((server) => server.name)
}

export function getConnectorCatalogDescription(): string {
  return CONNECTOR_CATALOG.map(
    (server) => `${server.name} (${server.description})`,
  ).join(', ')
}

export function isSupportedConnector(serverName: string): boolean {
  return CONNECTOR_CATALOG.some((server) => server.name === serverName)
}

export function findConnector(
  serverName: string,
): ConnectorCatalogItem | undefined {
  return CONNECTOR_CATALOG.find((server) => server.name === serverName)
}
