//! The TypeScript connector catalog, including its observable response order.

use super::Connector;

pub(super) const CATALOG: &[Connector] = &[
    Connector {
        name: "Gmail",
        description: "Send, read, and search emails",
    },
    Connector {
        name: "Google Calendar",
        description: "Create events, manage calendars",
    },
    Connector {
        name: "Google Docs",
        description: "Create and edit documents",
    },
    Connector {
        name: "Google Drive",
        description: "Upload, download, and manage files",
    },
    Connector {
        name: "Google Sheets",
        description: "Create and edit spreadsheets",
    },
    Connector {
        name: "Slack",
        description: "Post messages, manage channels",
    },
    Connector {
        name: "LinkedIn",
        description: "Post updates, manage connections",
    },
    Connector {
        name: "Notion",
        description: "Create pages, manage databases",
    },
    Connector {
        name: "Airtable",
        description: "Manage bases, tables, and records",
    },
    Connector {
        name: "Confluence",
        description: "Create and manage documentation",
    },
    Connector {
        name: "GitHub",
        description: "Manage repos, issues, pull requests",
    },
    Connector {
        name: "GitLab",
        description: "Manage repos, issues, merge requests",
    },
    Connector {
        name: "Linear",
        description: "Create issues, manage cycles",
    },
    Connector {
        name: "Jira",
        description: "Create issues, manage sprints",
    },
    Connector {
        name: "Figma",
        description: "Access and manage design files",
    },
    Connector {
        name: "Salesforce",
        description: "Manage leads, contacts, opportunities",
    },
    Connector {
        name: "ClickUp",
        description: "Manage tasks, projects, and workflows",
    },
    Connector {
        name: "Asana",
        description: "Organize and track team projects",
    },
    Connector {
        name: "Monday",
        description: "Manage work and team collaboration",
    },
    Connector {
        name: "Microsoft Teams",
        description: "Chat, meet, and collaborate",
    },
    Connector {
        name: "Outlook Mail",
        description: "Send, read, and manage emails",
    },
    Connector {
        name: "Outlook Calendar",
        description: "Schedule meetings and events",
    },
    Connector {
        name: "Supabase",
        description: "Manage databases and backend services",
    },
    Connector {
        name: "Vercel",
        description: "Deploy and manage web applications",
    },
    Connector {
        name: "Postman",
        description: "Test and manage APIs",
    },
    Connector {
        name: "Stripe",
        description: "Manage payments and subscriptions",
    },
    Connector {
        name: "Cloudflare",
        description: "Manage domains, DNS, and security",
    },
    Connector {
        name: "Brave Search",
        description: "Search the web privately",
    },
    Connector {
        name: "Mem0",
        description: "Store and retrieve AI memory",
    },
    Connector {
        name: "Dropbox",
        description: "Store and share files in the cloud",
    },
    Connector {
        name: "OneDrive",
        description: "Store and sync files with Microsoft",
    },
    Connector {
        name: "WordPress",
        description: "Manage websites and blog content",
    },
    Connector {
        name: "YouTube",
        description: "Access video info and transcripts",
    },
    Connector {
        name: "Box",
        description: "Manage and share enterprise files",
    },
    Connector {
        name: "HubSpot",
        description: "Manage contacts, deals, and marketing",
    },
    Connector {
        name: "PostHog",
        description: "Query analytics, manage feature flags",
    },
    Connector {
        name: "Mixpanel",
        description: "Analyze user behavior and metrics",
    },
    Connector {
        name: "Discord",
        description: "Send messages and manage servers",
    },
    Connector {
        name: "WhatsApp",
        description: "Send messages and manage conversations",
    },
    Connector {
        name: "Shopify",
        description: "Manage products, orders, and store",
    },
    Connector {
        name: "Cal.com",
        description: "Schedule meetings and manage availability",
    },
    Connector {
        name: "Resend",
        description: "Send transactional and marketing emails",
    },
    Connector {
        name: "Google Forms",
        description: "Create and manage forms and surveys",
    },
    Connector {
        name: "Zendesk",
        description: "Manage support tickets and customers",
    },
    Connector {
        name: "Intercom",
        description: "Manage customer messaging and support",
    },
];
