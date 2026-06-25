import { parse } from 'graphql'
import type { TypedDocumentString } from '@/generated/graphql/graphql'

const getOperationName = <T, V>(
  doc: TypedDocumentString<T, V>,
): string | null => {
  // Fallback to parsing
  const parsed = parse(doc.toString())
  const operation = parsed.definitions.find(
    (def) => def.kind === 'OperationDefinition',
  )

  return operation?.name ? operation.name.value : null
}

export const getQueryKeyFromDocument = <
  TResult,
  // biome-ignore lint/suspicious/noExplicitAny: TODO(dani) type GraphQL variables properly
  TVariables extends Record<string, any> | undefined = undefined,
>(
  doc: TypedDocumentString<TResult, TVariables>,
) => {
  const queryName = getOperationName(doc)
  return queryName
}
