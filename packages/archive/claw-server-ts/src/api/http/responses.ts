export interface BinaryAsset {
  bytes: Uint8Array
  etag?: string
}

export function binaryResponse(
  asset: BinaryAsset,
  cacheControl: string,
): Response {
  const headers: Record<string, string> = {
    'content-type': 'image/jpeg',
    'cache-control': cacheControl,
  }
  if (asset.etag) headers.etag = `"${asset.etag}"`
  // TS rejects `Uint8Array<ArrayBufferLike>` as BodyInit, so copy the
  // bytes into a fresh ArrayBuffer the Response can own.
  const body = new ArrayBuffer(asset.bytes.byteLength)
  new Uint8Array(body).set(asset.bytes)
  return new Response(body, { headers })
}
