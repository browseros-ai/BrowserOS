import type { Connection, ConnectionList } from '@browseros/claw-api'
import { Harness } from '@browseros/claw-api'
import type { Handler } from 'hono'
import {
  type ConnectionState,
  connectBrowserosToHarness,
  disconnectBrowserosFromHarness,
  listBrowserosConnections,
} from '../../../services/browseros-connect'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'

export interface ConnectionHandlerDependencies {
  listConnections(): Promise<ConnectionList>
  connectHarness(harness: Harness): Promise<Connection>
  disconnectHarness(harness: Harness): Promise<Connection>
}

export interface ConnectionHandlers {
  list: Handler<RequestContextEnv>
  connect: Handler<RequestContextEnv>
  disconnect: Handler<RequestContextEnv>
}

const harnesses = new Set<string>(Object.values(Harness))

export function createConnectionHandlers(
  dependencies: ConnectionHandlerDependencies,
): ConnectionHandlers {
  return {
    list: async (c) => c.json(await dependencies.listConnections()),
    connect: async (c) => {
      const harness = parseHarness(c.req.param('harness') ?? '')
      if (!harness) {
        return apiError(c, 404, 'harness_not_found', 'unknown harness')
      }
      return c.json(await dependencies.connectHarness(harness))
    },
    disconnect: async (c) => {
      const harness = parseHarness(c.req.param('harness') ?? '')
      if (!harness) {
        return apiError(c, 404, 'harness_not_found', 'unknown harness')
      }
      return c.json(await dependencies.disconnectHarness(harness))
    },
  }
}

export const productionConnectionHandlers = createConnectionHandlers({
  async listConnections() {
    return { items: (await listBrowserosConnections()).map(connection) }
  },
  async connectHarness(harness) {
    return connection(await connectBrowserosToHarness(harness))
  },
  async disconnectHarness(harness) {
    return connection(await disconnectBrowserosFromHarness(harness))
  },
})

function parseHarness(raw: string): Harness | null {
  let decoded: string
  try {
    decoded = decodeURIComponent(raw)
  } catch {
    return null
  }
  return harnesses.has(decoded) ? (decoded as Harness) : null
}

function connection(state: ConnectionState): Connection {
  return {
    harness: state.harness as Harness,
    installed: state.installed,
    ...(state.configPath === undefined ? {} : { configPath: state.configPath }),
    message: state.message,
  }
}
