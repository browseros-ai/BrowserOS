import {
  type UseMutationOptions,
  type UseMutationResult,
  useMutation,
} from '@tanstack/react-query'
import type { TypedDocumentString } from '@/generated/graphql/graphql'
import { execute } from '@/lib/graphql/execute'

/**
 * @public
 */
export function useGraphqlMutation<TResult = unknown, TVariables = object>(
  document: TypedDocumentString<TResult, TVariables>,
  options?: Omit<
    UseMutationOptions<TResult, unknown, TVariables>,
    'mutationFn' | 'mutationKey'
  >,
): UseMutationResult<TResult, unknown, TVariables> {
  return useMutation<TResult, unknown, TVariables>({
    mutationFn: (variables: TVariables) => execute(document, variables),
    ...(options ?? {}),
  })
}
