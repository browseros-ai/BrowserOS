/**
 * The Node Buffer view Playwright's protocol serializer needs in the agent realm.
 * The build injects only this module-local stand-in; URL, atob and btoa come from neo-sdk's
 * production realm, so serialized URL results share the agent's real URL constructor.
 */

/** `Buffer.from(arrayBuffer, byteOffset, length)`: a byte view, no copy, as Node's is. */
export const Buffer = {
  from: (buffer: ArrayBufferLike, byteOffset?: number, length?: number) =>
    new Uint8Array(buffer, byteOffset, length),
}
