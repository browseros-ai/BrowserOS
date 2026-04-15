import type { AclRule } from '@browseros/shared/types/acl'

type AclRulesResponse = {
  aclRules: AclRule[]
}

async function parseJsonResponse(
  response: Response,
): Promise<Record<string, unknown>> {
  return response.json().catch(() => ({}))
}

export async function fetchServerAclRules(baseUrl: string): Promise<AclRule[]> {
  const response = await fetch(`${baseUrl}/acl-rules`)
  if (!response.ok) {
    const data = await parseJsonResponse(response)
    throw new Error(String(data.error ?? `HTTP ${response.status}`))
  }

  const data = (await response.json()) as AclRulesResponse
  return data.aclRules
}

export async function updateServerAclRules(
  baseUrl: string,
  aclRules: AclRule[],
): Promise<AclRule[]> {
  const response = await fetch(`${baseUrl}/acl-rules`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ aclRules }),
  })
  if (!response.ok) {
    const data = await parseJsonResponse(response)
    throw new Error(String(data.error ?? `HTTP ${response.status}`))
  }

  const data = (await response.json()) as AclRulesResponse
  return data.aclRules
}
