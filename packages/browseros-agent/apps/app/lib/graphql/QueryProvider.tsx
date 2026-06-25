import { createAsyncStoragePersister } from '@tanstack/query-async-storage-persister'
import { QueryClient } from '@tanstack/react-query'
import {
  type AsyncStorage,
  PersistQueryClientProvider,
} from '@tanstack/react-query-persist-client'
import { del, get, set } from 'idb-keyval'
import type { FC, ReactNode } from 'react'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      gcTime: 1000 * 60 * 60 * 24, // 24 hours
    },
  },
})

const idbStorage: AsyncStorage<string> = {
  getItem: (key: string) => get<string>(key).then((v) => v ?? null),
  setItem: (key: string, value: string) => set(key, value),
  removeItem: (key: string) => del(key),
}

const asyncStoragePersister = createAsyncStoragePersister({
  storage: idbStorage,
})

export const QueryProvider: FC<{ children: ReactNode }> = ({ children }) => {
  return (
    <PersistQueryClientProvider
      persistOptions={{ persister: asyncStoragePersister }}
      client={queryClient}
    >
      {children}
    </PersistQueryClientProvider>
  )
}
