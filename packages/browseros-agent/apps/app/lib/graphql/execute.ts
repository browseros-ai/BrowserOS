import type { TypedDocumentString } from '@/generated/graphql/graphql'
import { env } from '../env'

export async function execute<TResult, TVariables = undefined>(
  query: TypedDocumentString<TResult, TVariables>,
  variables?: TVariables,
): Promise<TResult> {
  const headers = new Headers()
  headers.set('Content-Type', 'application/json')
  headers.set('Accept', 'application/graphql-response+json')

  const response = await fetch(`${env.VITE_PUBLIC_BROWSEROS_API}/graphql`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      query,
      variables,
    }),
    credentials: 'include',
  })

  if (!response.ok) {
    throw new Error(`Network response was not ok: ${response.statusText}`)
  }

  const body: { data?: TResult; errors?: { message: string }[] } =
    await response.json()

  if (body.errors && body.errors.length > 0) {
    const messages = body.errors.map((e) => e.message)
    throw new Error(`GraphQL error: ${messages.join(', ')}`)
  }

  if (!body.data) {
    throw new Error('GraphQL response is missing data.')
  }

  return body.data
}
