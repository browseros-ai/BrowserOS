import { z } from 'zod'
import type { BookmarkNode } from '../browser/bookmarks'
import { defineToolWithCategory } from './framework'

const defineManagementTool = defineToolWithCategory('data-modification')
const bookmarkNodeSchema = z.object({
  id: z.string(),
  title: z.string(),
  url: z.string().optional(),
  parentId: z.string().optional(),
  type: z.enum(['url', 'folder']),
  index: z.number().optional(),
  dateAdded: z.number(),
  dateLastUsed: z.number().optional(),
})

function formatBookmarkTree(nodes: BookmarkNode[]): string {
  const lines: string[] = []
  for (const node of nodes) {
    if (node.type === 'folder') {
      lines.push(`[${node.id}] ${node.title} (folder)`)
    } else {
      lines.push(`[${node.id}] ${node.title}`)
      lines.push(`    ${node.url}`)
    }
  }
  return lines.join('\n')
}

export const get_bookmarks = defineManagementTool({
  name: 'get_bookmarks',
  description: 'List all bookmarks in the browser',
  input: z.object({}),
  output: z.object({
    bookmarks: z.array(bookmarkNodeSchema),
    count: z.number(),
  }),
  handler: async (_args, ctx, response) => {
    const bookmarks = await ctx.browser.getBookmarks()
    if (bookmarks.length === 0) {
      response.text('No bookmarks found.')
      response.data({ bookmarks: [], count: 0 })
      return
    }
    response.text(
      `Found ${bookmarks.length} bookmarks:\n\n${formatBookmarkTree(bookmarks)}`,
    )
    response.data({ bookmarks, count: bookmarks.length })
  },
})

export const create_bookmark = defineManagementTool({
  name: 'create_bookmark',
  description: 'Create a new bookmark or folder. Omit url to create a folder.',
  input: z.object({
    title: z.string().describe('Bookmark title'),
    url: z
      .string()
      .optional()
      .describe('URL to bookmark (omit to create a folder)'),
    parentId: z.string().optional().describe('Folder ID to create bookmark in'),
  }),
  output: z.object({
    action: z.literal('create_bookmark'),
    bookmark: bookmarkNodeSchema,
  }),
  handler: async (args, ctx, response) => {
    const bookmark = await ctx.browser.createBookmark(args)
    if (bookmark.url) {
      response.text(
        `Created bookmark: ${bookmark.title}\nURL: ${bookmark.url}\nID: ${bookmark.id}`,
      )
    } else {
      response.text(`Created folder: ${bookmark.title}\nID: ${bookmark.id}`)
    }
    response.data({ action: 'create_bookmark', bookmark })
  },
})

export const remove_bookmark = defineManagementTool({
  name: 'remove_bookmark',
  description: 'Remove a bookmark or folder by ID (recursive)',
  input: z.object({
    id: z.string().describe('Bookmark or folder ID to remove'),
  }),
  output: z.object({
    action: z.literal('remove_bookmark'),
    id: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.removeBookmark(args.id)
    response.text(`Removed bookmark ${args.id}`)
    response.data({ action: 'remove_bookmark', id: args.id })
  },
})

export const update_bookmark = defineManagementTool({
  name: 'update_bookmark',
  description: 'Update a bookmark title or URL',
  input: z.object({
    id: z.string().describe('Bookmark ID to update'),
    title: z.string().optional().describe('New title for the bookmark'),
    url: z.string().optional().describe('New URL for the bookmark'),
  }),
  output: z.object({
    action: z.literal('update_bookmark'),
    bookmark: bookmarkNodeSchema,
  }),
  handler: async (args, ctx, response) => {
    const bookmark = await ctx.browser.updateBookmark(args.id, {
      title: args.title,
      url: args.url,
    })
    response.text(`Updated bookmark: ${bookmark.title}\nID: ${bookmark.id}`)
    response.data({ action: 'update_bookmark', bookmark })
  },
})

export const move_bookmark = defineManagementTool({
  name: 'move_bookmark',
  description: 'Move a bookmark or folder into a different folder',
  input: z.object({
    id: z.string().describe('Bookmark or folder ID to move'),
    parentId: z.string().optional().describe('Destination folder ID'),
    index: z
      .number()
      .int()
      .min(0)
      .optional()
      .describe('Position within parent (0-based)'),
  }),
  output: z.object({
    action: z.literal('move_bookmark'),
    bookmark: bookmarkNodeSchema,
  }),
  handler: async (args, ctx, response) => {
    const bookmark = await ctx.browser.moveBookmark(args.id, {
      parentId: args.parentId,
      index: args.index,
    })
    response.text(`Moved: ${bookmark.title}`)
    response.data({ action: 'move_bookmark', bookmark })
  },
})

export const search_bookmarks = defineManagementTool({
  name: 'search_bookmarks',
  description: 'Search bookmarks by title or URL',
  input: z.object({
    query: z
      .string()
      .describe('Search query to find bookmarks by title or URL'),
  }),
  output: z.object({
    query: z.string(),
    bookmarks: z.array(bookmarkNodeSchema),
    count: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const bookmarks = await ctx.browser.searchBookmarks(args.query)
    if (bookmarks.length === 0) {
      response.text(`No bookmarks found matching "${args.query}".`)
      response.data({ query: args.query, bookmarks: [], count: 0 })
      return
    }
    response.text(
      `Found ${bookmarks.length} bookmarks matching "${args.query}":\n\n${formatBookmarkTree(bookmarks)}`,
    )
    response.data({
      query: args.query,
      bookmarks,
      count: bookmarks.length,
    })
  },
})
