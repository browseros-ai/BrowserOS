import { type UseQueryOptions, useQuery } from '@tanstack/react-query'
import type { TypedDocumentString } from '@/generated/graphql/graphql'
import { execute } from '@/lib/graphql/execute'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'

/**
 * @public
 */
export const useGraphqlQuery = <
  TResult,
  // biome-ignore lint/suspicious/noExplicitAny: TODO(dani) type GraphQL variables properly
  TVariables extends Record<string, any> | undefined = undefined,
>(
  query: TypedDocumentString<TResult, TVariables>,
  variables?: TVariables,
  options?: Omit<UseQueryOptions<TResult, Error>, 'queryKey' | 'queryFn'>,
) => {
  const queryKey = getQueryKeyFromDocument(query)

  return useQuery<TResult, Error>({
    queryKey: variables ? [queryKey, variables] : [queryKey],
    queryFn: () => execute<TResult, TVariables>(query, variables),
    ...(options ?? {}),
  })
}
