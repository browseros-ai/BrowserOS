import { beforeEach, describe, expect, it } from 'bun:test'
import { createAclRoutes } from '../../../src/api/routes/acl'

describe('createAclRoutes', () => {
  let rules: Array<{
    id: string
    sitePattern: string
    selector?: string
    textMatch?: string
    description?: string
    enabled: boolean
  }>
  let route: ReturnType<typeof createAclRoutes>

  beforeEach(() => {
    rules = []
    route = createAclRoutes({
      policyService: {
        getRules: () => rules,
        setRules: async (next) => {
          rules = next
          return rules
        },
      } as never,
    })
  })

  it('returns the current global ACL rules', async () => {
    rules = [
      {
        id: 'checkout-submit',
        sitePattern: 'amazon.com',
        description: 'payments and checkout',
        enabled: true,
      },
    ]

    const response = await route.request('/')
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ aclRules: rules })
  })

  it('stores the provided ACL rules on put', async () => {
    const response = await route.request('/', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        aclRules: [
          {
            id: 'enabled',
            sitePattern: 'amazon.com',
            description: 'payments and checkout',
            enabled: true,
          },
          {
            id: 'disabled',
            sitePattern: 'amazon.com',
            description: 'disabled',
            enabled: false,
          },
        ],
      }),
    })

    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({
      aclRules: [
        {
          id: 'enabled',
          sitePattern: 'amazon.com',
          description: 'payments and checkout',
          enabled: true,
        },
        {
          id: 'disabled',
          sitePattern: 'amazon.com',
          description: 'disabled',
          enabled: false,
        },
      ],
    })
  })
})
