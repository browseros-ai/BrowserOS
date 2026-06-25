import type { InferResponseType } from 'hono/client'
import { createQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

// Destructured typed endpoints. The `$get` / `$post` accessors hc
// exposes for each route preserve the inferred request / response
// shape, which `InferResponseType` and `InferRequestType` unwrap.
const $health = api.system.health.$get
const $version = api.system.version.$get
const $url = api.system.url.$get

export type SystemHealth = InferResponseType<typeof $health>
export type SystemVersion = InferResponseType<typeof $version>
export type SystemUrl = InferResponseType<typeof $url>

export const useSystemHealth = createQuery<SystemHealth>({
  queryKey: ['system', 'health'],
  fetcher: () => $health().then(parseResponse<SystemHealth>),
})

export const useSystemVersion = createQuery<SystemVersion>({
  queryKey: ['system', 'version'],
  fetcher: () => $version().then(parseResponse<SystemVersion>),
})

export const useSystemUrl = createQuery<SystemUrl>({
  queryKey: ['system', 'url'],
  fetcher: () => $url().then(parseResponse<SystemUrl>),
  staleTime: 0,
  refetchOnMount: 'always',
})
