import { useCallback, useEffect, useState } from 'react'
import type { DiagnosticsResult } from './collector'
import { diagnosticSections, formatDiagnostics } from './contract'
import { requestDiagnostics } from './extension'
import './styles.css'

/** The same snapshot powers the visible rows and clipboard across both products. */
export function DiagnosticsPage() {
  const [result, setResult] = useState<DiagnosticsResult | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [copyFallback, setCopyFallback] = useState(false)
  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    setCopied(false)
    setCopyFallback(false)
    try {
      setResult(await requestDiagnostics())
    } catch (failure) {
      setError(
        failure instanceof Error ? failure.message : 'Diagnostics unavailable',
      )
    } finally {
      setLoading(false)
    }
  }, [])
  useEffect(() => {
    void refresh()
  }, [refresh])
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])

  const copy = () => {
    if (!result) return
    // Keep the clipboard write in the user's gesture; collection happens separately.
    try {
      void navigator.clipboard
        .writeText(formatDiagnostics(result.snapshot))
        .then(() => {
          setCopied(true)
          setCopyFallback(false)
        })
        .catch(() => setCopyFallback(true))
    } catch {
      setCopyFallback(true)
    }
  }

  return (
    <section
      className="browseros-diagnostics ph-no-capture"
      aria-labelledby="diagnostics-title"
    >
      <p className="diagnostics-breadcrumb">
        Help <span>/</span> Diagnostics
      </p>
      <div className="diagnostics-header">
        <div>
          <h1 id="diagnostics-title">Diagnostics</h1>
          <p>Version and system details to help us troubleshoot.</p>
        </div>
        <div className="diagnostics-actions">
          <button
            type="button"
            onClick={() => void refresh()}
            disabled={loading}
          >
            <span aria-hidden="true">↻</span>{' '}
            {loading ? 'Refreshing…' : 'Refresh'}
          </button>
          <button
            type="button"
            className="diagnostics-copy"
            onClick={copy}
            disabled={!result || loading}
          >
            {copied ? 'Copied' : 'Copy diagnostics'}
          </button>
        </div>
      </div>
      <p className="diagnostics-timestamp" role="status">
        {result
          ? `${result.cached ? 'Cached sample' : 'Collected'} · ${new Date(result.snapshot.collectedAt).toLocaleString()}`
          : loading
            ? 'Collecting diagnostics…'
            : 'No sample available'}
      </p>
      {error && (
        <p role="alert">
          {error}
          {result ? ' Showing the previous sample.' : ''}
        </p>
      )}
      {result &&
        diagnosticSections(result.snapshot).map((section) => (
          <section
            className="diagnostics-section"
            key={section.title}
            aria-label={section.title}
          >
            <div className="diagnostics-section-heading">
              <h2>{section.title}</h2>
              {section.title === 'Memory' && <span>At time of collection</span>}
            </div>
            <dl>
              {section.rows.map(([label, value]) => (
                <div className="diagnostics-row" key={label}>
                  <dt>{label}</dt>
                  <dd>{value ?? 'Unavailable'}</dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      {result && (
        <p className="diagnostics-note">
          Copy includes these details and the collection time. No browsing
          history or account information.
        </p>
      )}
      {copyFallback && result && (
        <label className="diagnostics-fallback">
          Clipboard unavailable. Select and copy these details:
          <textarea
            readOnly
            value={formatDiagnostics(result.snapshot)}
            onFocus={(event) => event.currentTarget.select()}
            rows={16}
          />
        </label>
      )}
    </section>
  )
}
