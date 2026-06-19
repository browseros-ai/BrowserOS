import { parse } from 'graphql'
import type { TypedDocumentString } from '@/generated/graphql/graphql'

const getOperationName = <TResult, TVariables>(
  doc: TypedDocumentString<TResult, TVariables>,
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
  TVariables extends Record<string, unknown> = Record<string, never>,
>(
  doc: TypedDocumentString<TResult, TVariables>,
) => {
  const queryName = getOperationName(doc)
  return queryName
}
