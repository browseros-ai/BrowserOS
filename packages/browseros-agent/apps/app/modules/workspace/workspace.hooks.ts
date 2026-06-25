import { useEffect, useState } from 'react'
import {
  selectedWorkspaceStorage,
  type WorkspaceFolder,
  workspaceFoldersStorage,
} from '@/lib/workspace/workspace-storage'

const MAX_RECENT_FOLDERS = 10

/**
 * @public
 */
export function useWorkspace() {
  const [recentFolders, setRecentFolders] = useState<WorkspaceFolder[]>([])
  const [selectedFolder, setSelectedFolder] = useState<WorkspaceFolder | null>(
    null,
  )

  useEffect(() => {
    workspaceFoldersStorage.getValue().then(setRecentFolders)
    selectedWorkspaceStorage.getValue().then(setSelectedFolder)

    const unwatchRecent = workspaceFoldersStorage.watch((value) => {
      setRecentFolders(value ?? [])
    })
    const unwatchSelected = selectedWorkspaceStorage.watch((value) => {
      setSelectedFolder(value)
    })

    return () => {
      unwatchRecent()
      unwatchSelected()
    }
  }, [])

  const selectFolder = async (folder: WorkspaceFolder) => {
    await selectedWorkspaceStorage.setValue(folder)

    const current = (await workspaceFoldersStorage.getValue()) ?? []
    const filtered = current.filter((f) => f.path !== folder.path)
    const updated = [{ ...folder, addedAt: Date.now() }, ...filtered].slice(
      0,
      MAX_RECENT_FOLDERS,
    )
    await workspaceFoldersStorage.setValue(updated)
  }

  const addFolder = async (folder: WorkspaceFolder) => {
    await selectFolder(folder)
  }

  const removeFolder = async (id: string) => {
    const current = (await workspaceFoldersStorage.getValue()) ?? []
    await workspaceFoldersStorage.setValue(current.filter((f) => f.id !== id))

    const selected = await selectedWorkspaceStorage.getValue()
    if (selected?.id === id) {
      await selectedWorkspaceStorage.setValue(null)
    }
  }

  const clearSelection = async () => {
    await selectedWorkspaceStorage.setValue(null)
  }

  return {
    recentFolders,
    selectedFolder,
    selectFolder,
    addFolder,
    removeFolder,
    clearSelection,
  }
}
