/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Agent-facing tools for extension management.
 * Delegates to browser/extensions.ts module via Browser class methods.
 */

import { z } from 'zod'
import { defineToolWithCategory } from './framework'
import type { StorageArea } from '@browseros/cdp-protocol/generated/domains/extensions'

const defineExtTool = defineToolWithCategory('data-modification')

// ── Lifecycle tools ──

export const load_unpacked_extension = defineExtTool({
  name: 'load_unpacked_extension',
  description:
    'Load an unpacked extension from a local directory path. Returns the extension ID.',
  input: z.object({
    path: z.string().describe('Absolute path to the extension directory'),
  }),
  output: z.object({
    action: z.literal('load_unpacked'),
    id: z.string(),
    path: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const id = await ctx.browser.loadUnpackedExtension(args.path)
    response.text(`Loaded extension from ${args.path}\nExtension ID: ${id}`)
    response.data({ action: 'load_unpacked', id, path: args.path })
  },
})

export const uninstall_extension = defineExtTool({
  name: 'uninstall_extension',
  description:
    'Uninstall an extension by ID. Cannot uninstall BrowserOS first-party extensions.',
  input: z.object({
    extensionId: z.string().describe('Extension ID to uninstall'),
  }),
  output: z.object({
    action: z.literal('uninstall'),
    extensionId: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.uninstallExtension(args.extensionId)
    response.text(`Uninstalled extension ${args.extensionId}`)
    response.data({ action: 'uninstall', extensionId: args.extensionId })
  },
})

// ── Storage tools ──

export const get_extension_storage = defineExtTool({
  name: 'get_extension_storage',
  description:
    'Get storage items from an extension. Supports local, sync, session, and managed (read-only) areas.',
  input: z.object({
    extensionId: z
      .string()
      .describe('Extension ID to read storage from'),
    storageArea: z
      .enum(['local', 'sync', 'session', 'managed'])
      .describe('Storage area to read'),
    keys: z
      .array(z.string())
      .optional()
      .describe('Specific keys to retrieve. Omit for all keys.'),
  }),
  output: z.object({
    action: z.literal('get_storage'),
    extensionId: z.string(),
    storageArea: z.string(),
    data: z.record(z.unknown()),
  }),
  handler: async (args, ctx, response) => {
    const data = await ctx.browser.getExtensionStorage(
      args.extensionId,
      args.storageArea as StorageArea,
      args.keys,
    )
    response.text(
      `Storage for ${args.extensionId} (${args.storageArea}): ${JSON.stringify(data, null, 2)}`,
    )
    response.data({
      action: 'get_storage',
      extensionId: args.extensionId,
      storageArea: args.storageArea,
      data,
    })
  },
})

export const set_extension_storage = defineExtTool({
  name: 'set_extension_storage',
  description:
    'Set storage items for an extension. Supports local, sync, and session areas. Managed storage is read-only.',
  input: z.object({
    extensionId: z
      .string()
      .describe('Extension ID to write storage to'),
    storageArea: z
      .enum(['local', 'sync', 'session'])
      .describe('Storage area to write'),
    values: z
      .record(z.unknown())
      .describe('Key-value pairs to set'),
  }),
  output: z.object({
    action: z.literal('set_storage'),
    extensionId: z.string(),
    storageArea: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.setExtensionStorage(
      args.extensionId,
      args.storageArea as StorageArea,
      args.values,
    )
    response.text(
      `Set storage for ${args.extensionId} (${args.storageArea}): ${Object.keys(args.values).join(', ')}`,
    )
    response.data({
      action: 'set_storage',
      extensionId: args.extensionId,
      storageArea: args.storageArea,
    })
  },
})

export const remove_extension_storage = defineExtTool({
  name: 'remove_extension_storage',
  description:
    'Remove specific keys from an extension storage area. Managed storage is read-only.',
  input: z.object({
    extensionId: z
      .string()
      .describe('Extension ID'),
    storageArea: z
      .enum(['local', 'sync', 'session'])
      .describe('Storage area'),
    keys: z
      .array(z.string())
      .describe('Keys to remove'),
  }),
  output: z.object({
    action: z.literal('remove_storage'),
    extensionId: z.string(),
    storageArea: z.string(),
    removedKeys: z.array(z.string()),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.removeExtensionStorage(
      args.extensionId,
      args.storageArea as StorageArea,
      args.keys,
    )
    response.text(
      `Removed keys [${args.keys.join(', ')}] from ${args.extensionId} (${args.storageArea})`,
    )
    response.data({
      action: 'remove_storage',
      extensionId: args.extensionId,
      storageArea: args.storageArea,
      removedKeys: args.keys,
    })
  },
})

export const clear_extension_storage = defineExtTool({
  name: 'clear_extension_storage',
  description:
    'Clear all items from an extension storage area. Managed storage is read-only.',
  input: z.object({
    extensionId: z
      .string()
      .describe('Extension ID'),
    storageArea: z
      .enum(['local', 'sync', 'session'])
      .describe('Storage area to clear'),
  }),
  output: z.object({
    action: z.literal('clear_storage'),
    extensionId: z.string(),
    storageArea: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.clearExtensionStorage(
      args.extensionId,
      args.storageArea as StorageArea,
    )
    response.text(
      `Cleared ${args.storageArea} storage for ${args.extensionId}`,
    )
    response.data({
      action: 'clear_storage',
      extensionId: args.extensionId,
      storageArea: args.storageArea,
    })
  },
})

// ── Placeholder tools for L2 CDP handler (list/enable/disable/get_info) ──
// These require the new Chromium CDP handler — stubs for now.

export const list_extensions = defineExtTool({
  name: 'list_extensions',
  description:
    'List all installed extensions. Returns id, name, version, state, canModify, isBrowserOS. Requires L2 CDP handler.',
  input: z.object({}),
  output: z.object({
    extensions: z.array(
      z.object({
        id: z.string(),
        name: z.string(),
        version: z.string(),
        state: z.string(),
        canModify: z.boolean(),
        isBrowserOS: z.boolean(),
      }),
    ),
    count: z.number(),
  }),
  handler: async (_args, _ctx, response) => {
    // TODO: Requires L2 CDP handler (Extensions.listExtensions)
    response.text('Extension listing requires L2 CDP handler (not yet implemented)')
    response.data({ extensions: [], count: 0 })
  },
})

export const get_extension_info = defineExtTool({
  name: 'get_extension_info',
  description:
    'Get detailed info for a specific extension. Requires L2 CDP handler.',
  input: z.object({
    extensionId: z.string().describe('Extension ID'),
  }),
  output: z.object({
    id: z.string(),
    name: z.string(),
    state: z.string(),
    canModify: z.boolean(),
  }),
  handler: async (_args, _ctx, response) => {
    // TODO: Requires L2 CDP handler
    response.text('Extension info requires L2 CDP handler (not yet implemented)')
    response.data({ id: '', name: '', state: '', canModify: false })
  },
})

export const enable_extension = defineExtTool({
  name: 'enable_extension',
  description:
    'Enable a disabled extension. Requires L2 CDP handler.',
  input: z.object({
    extensionId: z.string().describe('Extension ID to enable'),
  }),
  output: z.object({
    action: z.literal('enable'),
    extensionId: z.string(),
  }),
  handler: async (_args, _ctx, response) => {
    // TODO: Requires L2 CDP handler
    response.text('Enable extension requires L2 CDP handler (not yet implemented)')
    response.data({ action: 'enable', extensionId: '' })
  },
})

export const disable_extension = defineExtTool({
  name: 'disable_extension',
  description:
    'Disable an enabled extension. Cannot disable BrowserOS first-party extensions. Requires L2 CDP handler.',
  input: z.object({
    extensionId: z.string().describe('Extension ID to disable'),
  }),
  output: z.object({
    action: z.literal('disable'),
    extensionId: z.string(),
  }),
  handler: async (_args, _ctx, response) => {
    // TODO: Requires L2 CDP handler
    response.text('Disable extension requires L2 CDP handler (not yet implemented)')
    response.data({ action: 'disable', extensionId: '' })
  },
})
