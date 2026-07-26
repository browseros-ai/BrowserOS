import { storage } from '@wxt-dev/storage'

/**
 * @public
 */
export interface WorkspaceFolder {
  id: string
  name: string
  path: string
  addedAt: number
}

export const workspaceFoldersStorage = storage.defineItem<WorkspaceFolder[]>(
  'local:workspaceFolders',
  { fallback: [] },
)

export const selectedWorkspaceStorage =
  storage.defineItem<WorkspaceFolder | null>('local:selectedWorkspace', {
    fallback: null,
  })
