import { createContext, type FC, type ReactNode, use, useMemo } from 'react'
import { getClient, type RpcClient } from './getClient'

const RpcClientContext = createContext<Promise<RpcClient> | null>(null)

/**
 * @public
 */
export const RpcClientProvider: FC<{ children: ReactNode }> = ({
  children,
}) => {
  const clientPromise = useMemo(() => getClient(), [])
  return (
    <RpcClientContext.Provider value={clientPromise}>
      {children}
    </RpcClientContext.Provider>
  )
}

/**
 * @public
 */
export function useRpcClient(): RpcClient {
  const promise = use(RpcClientContext)
  if (!promise) {
    throw new Error('useRpcClient must be used within RpcClientProvider')
  }
  return use(promise)
}
