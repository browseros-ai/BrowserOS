/**
 * The push door: `POST /otlp/v1/traces` and `POST /otlp/v1/metrics`, OTLP/HTTP JSON, plain or
 * gzip. Neo POSTs here when started with `OTEL_EXPORTER_OTLP_ENDPOINT=<viewer>/otlp` and
 * `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`; the viewer adds no protobuf library, so a protobuf
 * body is refused with a 415 that names the setting to change.
 *
 * Not behind the page's `local()` guard: a container reaches the viewer's 127.0.0.1 through
 * Docker Desktop's `host.docker.internal`, and its requests say so in `Host` (probed on this
 * machine). These routes take writes only, and keep their own guard:
 * - `Host` is a loopback name or `host.docker.internal`, so a page that rebinds its own domain to
 *   127.0.0.1 is refused;
 * - no `Origin`: every browser sends one on a POST, an OTel exporter never does;
 * - `Content-Type: application/json` only, so a cross-site form (text/plain) cannot land, and the
 *   viewer never answers a CORS preflight;
 * - at most 4 MiB, after gunzip too, read in pieces: a large or zip-bomb body is cut off, never
 *   held whole.
 * Success is OTLP's `Export*ServiceResponse`: `{}`, or `partialSuccess` naming what was dropped.
 */

import { gunzipSync } from 'node:zlib'
import type { MetricFamily } from '../../shared/api'
import {
  type Decoded,
  decodeMetrics,
  decodeSpans,
  type NeoSpan,
  NotOtlp,
} from './otlp-json'

/** Where accepted data goes; `spans` says how many it refused (a run over its cap). */
export interface ReceiverSinks {
  spans(spans: NeoSpan[]): { rejected: number }
  metrics(families: MetricFamily[]): void
}

export const OTLP_TRACES_PATH = '/otlp/v1/traces'
export const OTLP_METRICS_PATH = '/otlp/v1/metrics'
export const MAX_OTLP_BYTES = 4 * 1024 * 1024

const HOSTS = new Set([
  '127.0.0.1',
  'localhost',
  '[::1]',
  'host.docker.internal',
])

type Handler = (req: Request) => Promise<Response>

export function receiverRoutes(
  sinks: ReceiverSinks,
  maxBytes = MAX_OTLP_BYTES,
): Record<string, { POST: Handler }> {
  return {
    [OTLP_TRACES_PATH]: {
      POST: (req) =>
        receive(req, maxBytes, (body) => {
          const decoded = decodeSpans(body)
          const { rejected } = sinks.spans(decoded.items)
          return partial('Spans', decoded, rejected)
        }),
    },
    [OTLP_METRICS_PATH]: {
      POST: (req) =>
        receive(req, maxBytes, (body) => {
          const decoded = decodeMetrics(body)
          if (decoded.items.length > 0) sinks.metrics(decoded.items)
          return partial('DataPoints', decoded, 0)
        }),
    },
  }
}

async function receive(
  req: Request,
  maxBytes: number,
  accept: (body: unknown) => Record<string, unknown>,
): Promise<Response> {
  if (!HOSTS.has(hostName(req))) return refuse(403, 'forbidden host')
  if (req.headers.has('origin'))
    return refuse(
      403,
      'forbidden origin: the OTLP receiver takes no browser requests',
    )
  const type = (req.headers.get('content-type') ?? '')
    .split(';')[0]
    ?.trim()
    .toLowerCase()
  if (type === 'application/x-protobuf' || type === 'application/protobuf')
    return refuse(
      415,
      'this receiver reads OTLP/HTTP JSON only, not protobuf: set OTEL_EXPORTER_OTLP_PROTOCOL=http/json',
    )
  if (type !== 'application/json')
    return refuse(
      415,
      'send OTLP/HTTP JSON with Content-Type: application/json',
    )
  const coding = (req.headers.get('content-encoding') ?? 'identity')
    .trim()
    .toLowerCase()
  if (coding !== 'identity' && coding !== 'gzip')
    return refuse(
      415,
      `unsupported Content-Encoding ${coding}: send gzip or none`,
    )
  const raw = await readCapped(req, maxBytes)
  if (raw === null) return refuse(413, `the body is over ${maxBytes} bytes`)
  let bytes: Uint8Array = raw
  if (coding === 'gzip') {
    try {
      bytes = gunzipSync(raw, { maxOutputLength: maxBytes })
    } catch (error) {
      return (error as NodeJS.ErrnoException).code === 'ERR_BUFFER_TOO_LARGE'
        ? refuse(413, `the body is over ${maxBytes} bytes once gunzipped`)
        : refuse(400, `the body is not gzip: ${String(error)}`)
    }
  }
  let body: unknown
  try {
    body = JSON.parse(new TextDecoder().decode(bytes))
  } catch (error) {
    return refuse(400, `malformed OTLP JSON: ${String(error)}`)
  }
  try {
    return Response.json(accept(body))
  } catch (error) {
    if (error instanceof NotOtlp)
      return refuse(400, `not an OTLP export request: ${error.message}`)
    throw error
  }
}

/** `{}`, or `partialSuccess` with the count as OTLP JSON writes an int64: a string. */
function partial(
  what: 'Spans' | 'DataPoints',
  decoded: Decoded<unknown>,
  refused: number,
): Record<string, unknown> {
  const rejected = decoded.rejected + refused
  if (rejected === 0) return {}
  return {
    partialSuccess: {
      [`rejected${what}`]: String(rejected),
      errorMessage:
        what === 'Spans'
          ? 'only neo.run and operation spans (neo.span.type) that name their run are kept; a run keeps at most 2000'
          : 'data points without a value were dropped',
    },
  }
}

/** The request body, or null once it passes `max` bytes (read in pieces, never whole). */
async function readCapped(
  req: Request,
  max: number,
): Promise<Uint8Array | null> {
  if (Number(req.headers.get('content-length') ?? 0) > max) return null
  const chunks: Uint8Array[] = []
  let size = 0
  for await (const chunk of req.body ?? []) {
    size += chunk.byteLength
    if (size > max) return null
    chunks.push(chunk)
  }
  return Buffer.concat(chunks)
}

/** The host name `Host` names, without its port (`[::1]` keeps its brackets). */
function hostName(req: Request): string {
  const host = (req.headers.get('host') ?? '').toLowerCase()
  return host.startsWith('[')
    ? host.slice(0, host.indexOf(']') + 1)
    : (host.split(':')[0] ?? '')
}

/** An OTLP failure: a JSON `Status` (only `message`; OTLP leaves `code` to gRPC). */
function refuse(status: number, message: string): Response {
  return Response.json({ message: `neo-viewer: ${message}` }, { status })
}
