/**
 * Neo's two doors name its metrics differently. The scrape (`GET /v1/metrics`) serves Prometheus
 * names and label names (`neo_run_duration_seconds{status}`); the OTLP push sends OpenTelemetry's
 * (`neo.run.duration` with `neo.run.status`). The viewer's model is the scrape's, so a pushed
 * metric is renamed into it here and the Health view and health bar read one set of names
 * whichever door answered (openapi.yaml, `/v1/metrics`).
 */

import type { MetricFamily } from '../../shared/api'

/** Each OTLP metric name, and the scrape family it is. */
export const SCRAPE_FAMILY: Readonly<Record<string, string>> = {
  'neo.runs': 'neo_runs_total',
  'neo.run.active': 'neo_runs_in_flight',
  'neo.run.duration': 'neo_run_duration_seconds',
  'neo.operations': 'neo_operations_total',
  'neo.operation.duration': 'neo_operation_duration_seconds',
  'neo.errors': 'neo_errors_total',
  'neo.tab.count': 'neo_tabs_open',
  'neo.tab.opened': 'neo_tabs_opened_total',
  'neo.tab.closed': 'neo_tabs_closed_total',
  'neo.tab.duration': 'neo_tab_lifetime_seconds',
  'neo.renderer.crashes': 'neo_renderer_crashes_total',
  'neo.navigations': 'neo_navigations_total',
  'neo.navigation.errors': 'neo_navigation_errors_total',
  'neo.captcha.solves': 'neo_captcha_solves_total',
  'neo.captcha.reasons': 'neo_captcha_reasons_total',
  'neo.captcha.spend': 'neo_captcha_spend_usd_total',
}

/** Each OTLP attribute key Neo's metrics carry, and the scrape's label name for it. */
export const SCRAPE_LABEL: Readonly<Record<string, string>> = {
  'neo.run.status': 'status',
  'neo.op.outcome': 'status',
  'neo.op.name': 'op',
  'neo.error.owner': 'error_owner',
  'error.type': 'error_code',
  'neo.tab.origin': 'origin',
  'neo.tab.close_reason': 'close_reason',
  'http.response.status_code': 'status_code',
  'neo.captcha.family': 'family',
  'neo.captcha.verdict': 'verdict',
  'neo.captcha.reason': 'reason',
  'neo.captcha.provider': 'provider',
}

/**
 * A pushed family as the scrape names it: its name and every series' labels. Only Neo's own
 * metrics are renamed: any other family is kept as sent.
 */
export function asScraped(family: MetricFamily): MetricFamily {
  const name = lookup(SCRAPE_FAMILY, family.name)
  if (name === undefined) return family
  return {
    ...family,
    name,
    series: family.series.map((series) => ({
      ...series,
      labels: Object.fromEntries(
        Object.entries(series.labels).map(([key, value]) => [
          lookup(SCRAPE_LABEL, key) ?? key,
          value,
        ]),
      ),
    })),
  }
}

function lookup(
  table: Readonly<Record<string, string>>,
  key: string,
): string | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined
}
