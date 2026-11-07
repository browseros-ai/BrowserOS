/**
 * Playwright: `client/credentials.ts` `Credentials`
 *
 * `context.credentials`: the run's virtual WebAuthn credentials (neo-runtime `credentials.rs`),
 * served by Chromium's own virtual authenticator on each page once installed. Every call goes
 * out as Playwright's does, without a timeout.
 */

import type {
  BrowserContextCredentialsCreateParams,
  BrowserContextCredentialsGetParams,
  VirtualCredential,
} from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'

export class Credentials {
  readonly #connection: Connection
  readonly #guid: string

  constructor(connection: Connection, guid: string) {
    this.#connection = connection
    this.#guid = guid
  }

  async install(): Promise<void> {
    await this.#connection.sendContext(
      'credentials.install',
      this.#guid,
      'credentialsInstall',
      {},
      0,
    )
  }

  async create(
    rpId: string,
    options: Omit<BrowserContextCredentialsCreateParams, 'rpId'> = {},
  ): Promise<VirtualCredential> {
    const { credential } = await this.#connection.sendContext(
      'credentials.create',
      this.#guid,
      'credentialsCreate',
      { ...options, rpId },
      0,
    )
    return credential
  }

  async get(
    options: BrowserContextCredentialsGetParams = {},
  ): Promise<VirtualCredential[]> {
    const { credentials } = await this.#connection.sendContext(
      'credentials.get',
      this.#guid,
      'credentialsGet',
      options,
      0,
    )
    return credentials
  }

  async delete(id: string): Promise<void> {
    await this.#connection.sendContext(
      'credentials.delete',
      this.#guid,
      'credentialsDelete',
      { id },
      0,
    )
  }
}
