/**
 * Weekly Report Generator
 *
 * Reads all uploaded eval runs from R2, builds cumulative score history,
 * and generates an HTML dashboard with:
 *   - Config selector dropdown (groups runs by config/runId pattern)
 *   - Config details card (architecture, model, dataset, grader)
 *   - Interactive trend chart (filtered by selected config)
 *   - Stat cards (latest, trend, best, duration)
 *   - Searchable table of all runs
 *
 * Usage:
 *   bun apps/eval/scripts/weekly-report.ts [local-output-path]
 *
 * Env vars required:
 *   EVAL_R2_ACCOUNT_ID, EVAL_R2_ACCESS_KEY_ID, EVAL_R2_SECRET_ACCESS_KEY
 *   EVAL_R2_BUCKET (default: browseros-eval)
 */

import { writeFile } from 'node:fs/promises'
import {
  GetObjectCommand,
  ListObjectsV2Command,
  PutObjectCommand,
  S3Client,
} from '@aws-sdk/client-s3'

interface ManifestTask {
  queryId: string
  query: string
  status: string
  durationMs: number
  screenshotCount: number
  graderResults: Record<string, { pass: boolean; score: number }>
}

interface Manifest {
  runId: string
  uploadedAt: string
  agentConfig?: { type?: string; model?: string }
  dataset?: string
  summary?: { passRate?: number; avgDurationMs?: number }
  tasks: ManifestTask[]
}

interface RunSummary {
  runId: string
  configName: string
  date: string
  passRate: number
  total: number
  completed: number
  failed: number
  timeout: number
  avgDurationMs: number
  model: string
  dataset: string
  agentType: string
}

const PASS_FAIL_GRADER_ORDER = [
  'performance_grader',
  'webvoyager_grader',
  'fara_combined',
  'fara_grader',
]

function requireEnv(name: string): string {
  const value = process.env[name]
  if (!value) {
    console.error(`Missing required env var: ${name}`)
    process.exit(1)
  }
  return value
}

const accountId = requireEnv('EVAL_R2_ACCOUNT_ID')
const accessKeyId = requireEnv('EVAL_R2_ACCESS_KEY_ID')
const secretAccessKey = requireEnv('EVAL_R2_SECRET_ACCESS_KEY')
const bucket = process.env.EVAL_R2_BUCKET || 'browseros-eval'

const client = new S3Client({
  region: 'auto',
  endpoint: `https://${accountId}.r2.cloudflarestorage.com`,
  credentials: { accessKeyId, secretAccessKey },
})

// Step 1: List all manifest.json files in runs/
console.log('Scanning R2 for eval runs...')

const manifests: Manifest[] = []
let continuationToken: string | undefined

do {
  const listRes = await client.send(
    new ListObjectsV2Command({
      Bucket: bucket,
      Prefix: 'runs/',
      ContinuationToken: continuationToken,
    }),
  )

  const manifestKeys =
    listRes.Contents?.filter((obj) => obj.Key?.endsWith('/manifest.json')).map(
      (obj) => obj.Key as string,
    ) ?? []

  for (const key of manifestKeys) {
    try {
      const res = await client.send(
        new GetObjectCommand({ Bucket: bucket, Key: key }),
      )
      const body = await res.Body?.transformToString()
      if (body) manifests.push(JSON.parse(body))
    } catch {
      console.warn(`  Failed to read ${key}, skipping`)
    }
  }

  continuationToken = listRes.NextContinuationToken
} while (continuationToken)

console.log(`Found ${manifests.length} runs`)

if (manifests.length === 0) {
  console.log('No runs found. Nothing to report.')
  process.exit(0)
}

// Step 2: Build run summaries
const runs: RunSummary[] = manifests
  .map((m) => {
    const total = m.tasks.length
    const completed = m.tasks.filter((t) => t.status === 'completed').length
    const failed = m.tasks.filter((t) => t.status === 'failed').length
    const timeout = m.tasks.filter((t) => t.status === 'timeout').length

    let graded = 0
    let passed = 0
    for (const task of m.tasks) {
      if (!task.graderResults) continue
      for (const name of PASS_FAIL_GRADER_ORDER) {
        if (task.graderResults[name]) {
          graded++
          if (task.graderResults[name].pass) passed++
          break
        }
      }
    }

    const passRate = graded > 0 ? passed / graded : 0
    const durations = m.tasks
      .filter((t) => t.durationMs > 0)
      .map((t) => t.durationMs)
    const avgDurationMs =
      durations.length > 0
        ? durations.reduce((a, b) => a + b, 0) / durations.length
        : 0

    const date = m.uploadedAt
      ? `${m.uploadedAt.split('T')[0]} ${m.uploadedAt.split('T')[1]?.slice(0, 5) || ''}`
      : m.runId.slice(0, 15)

    const model = m.agentConfig?.model || 'unknown'
    const dataset = m.dataset || m.runId
    const agentType = m.agentConfig?.type || 'unknown'

    const configName = extractConfigName(m.runId)
    return {
      runId: m.runId,
      configName,
      date,
      passRate,
      total,
      completed,
      failed,
      timeout,
      avgDurationMs,
      model,
      dataset,
      agentType,
    }
  })
  .sort((a, b) => a.date.localeCompare(b.date))

// Step 3: Identify unique config groups
// runId can be "ci-weekly" (old) or "ci-weekly-2026-03-21-1730" (timestamped)
// Extract config name by stripping the date-time suffix pattern
function escHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function extractConfigName(runId: string): string {
  // "browseros-agent-weekly-2026-03-21-1730" → "browseros-agent-weekly"
  // "ci-weekly" → "ci-weekly" (no timestamp, old format)
  return runId.replace(/-\d{4}-\d{2}-\d{2}-\d{4}$/, '')
}

const configGroups = [...new Set(runs.map((r) => r.configName))]
const defaultConfig = configGroups.includes('ci-weekly')
  ? 'ci-weekly'
  : configGroups[0]

// Step 4: Generate HTML report
const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>BrowserOS Eval Dashboard</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif; background: #0d1117; color: #e6edf3; padding: 2rem; max-width: 1400px; margin: 0 auto; }

    /* Header */
    .page-header { display: flex; align-items: center; gap: 16px; margin-bottom: 2rem; flex-wrap: wrap; }
    .page-header h1 { font-size: 1.5rem; }
    .page-header h1 span { color: #58a6ff; }
    .page-header .gen-date { color: #6e7681; font-size: 12px; margin-left: auto; }

    /* Config selector */
    .config-bar { display: flex; align-items: center; gap: 16px; margin-bottom: 1.5rem; flex-wrap: wrap; }
    .config-bar label { font-size: 13px; color: #8b949e; font-weight: 600; }
    .config-bar select { background: #161b22; border: 1px solid #30363d; color: #e6edf3; padding: 8px 12px; border-radius: 6px; font-size: 13px; font-family: 'SF Mono', Consolas, monospace; cursor: pointer; min-width: 200px; }
    .config-bar select:focus { outline: none; border-color: #58a6ff; }

    /* Config details card */
    .config-details { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 16px 20px; margin-bottom: 1.5rem; display: flex; gap: 32px; flex-wrap: wrap; }
    .config-detail { display: flex; flex-direction: column; gap: 2px; }
    .config-detail .cd-label { font-size: 10px; font-weight: 600; color: #6e7681; text-transform: uppercase; letter-spacing: 0.04em; }
    .config-detail .cd-value { font-size: 13px; color: #e6edf3; font-family: 'SF Mono', Consolas, monospace; }

    /* Stat cards */
    .stats { display: flex; gap: 1rem; margin-bottom: 1.5rem; flex-wrap: wrap; }
    .stat-card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 1.25rem; flex: 1; min-width: 140px; }
    .stat-label { color: #8b949e; font-size: 0.8rem; margin-bottom: 0.25rem; }
    .stat-value { font-size: 1.4rem; font-weight: 600; }
    .stat-value.big { font-size: 2.5rem; font-weight: 700; }
    .pass { color: #3fb950; }
    .fail { color: #f85149; }
    .neutral { color: #8b949e; }
    .trend-up { color: #3fb950; }
    .trend-down { color: #f85149; }
    .trend-flat { color: #8b949e; }

    /* Chart */
    .chart-container { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 1.5rem; margin-bottom: 2rem; position: relative; }
    canvas { width: 100%; height: 300px; }
    #tooltip { display: none; position: absolute; background: #1c2128; border: 1px solid #30363d; border-radius: 6px; padding: 8px 12px; pointer-events: none; font-size: 12px; z-index: 10; box-shadow: 0 4px 12px rgba(0,0,0,0.4); }

    /* Section headers */
    .section-header { display: flex; align-items: center; gap: 12px; margin-bottom: 1rem; }
    .section-header h2 { font-size: 1rem; font-weight: 600; }
    .section-header .search-input { margin-left: auto; background: #0d1117; border: 1px solid #30363d; color: #e6edf3; padding: 6px 12px; border-radius: 6px; font-size: 12px; font-family: inherit; width: 220px; }
    .section-header .search-input:focus { outline: none; border-color: #58a6ff; }
    .section-header .search-input::placeholder { color: #484f58; }

    /* Table */
    table { width: 100%; border-collapse: collapse; background: #161b22; border: 1px solid #30363d; border-radius: 8px; overflow: hidden; }
    th, td { padding: 0.65rem 1rem; text-align: left; border-bottom: 1px solid #21262d; }
    th { background: #1c2128; color: #8b949e; font-weight: 600; font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.03em; }
    td { font-size: 0.85rem; }
    td.mono { font-family: 'SF Mono', Consolas, monospace; font-size: 0.8rem; }
    a.view-link { color: #58a6ff; text-decoration: none; font-weight: 500; }
    a.view-link:hover { text-decoration: underline; }
    tr.hidden { display: none; }
  </style>
</head>
<body>

<div class="page-header">
  <h1>BrowserOS <span>Eval Dashboard</span></h1>
  <span class="gen-date">Generated ${new Date().toISOString().split('T')[0]}</span>
</div>

<!-- Config selector -->
<div class="config-bar">
  <label>Config:</label>
  <select id="config-select">
    ${configGroups.map((c) => `<option value="${escHtml(c)}"${c === defaultConfig ? ' selected' : ''}>${escHtml(c)}</option>`).join('\n    ')}
  </select>
</div>

<!-- Config details (populated by JS) -->
<div class="config-details" id="config-details"></div>

<!-- Stat cards (populated by JS) -->
<div class="stats" id="stat-cards"></div>

<!-- Chart -->
<div class="chart-container">
  <canvas id="chart"></canvas>
  <div id="tooltip">
    <div id="tt-date" style="color:#8b949e;margin-bottom:2px;"></div>
    <div id="tt-score" style="font-size:16px;font-weight:700;"></div>
    <div id="tt-detail" style="color:#8b949e;margin-top:2px;font-size:11px;"></div>
  </div>
</div>

<!-- Recent runs table -->
<div class="section-header">
  <h2>All Runs</h2>
  <input type="text" class="search-input" id="table-search" placeholder="Search runs..." autocomplete="off" spellcheck="false" />
</div>
<table id="runs-table">
  <thead>
    <tr>
      <th>Date</th>
      <th>Config</th>
      <th>Model</th>
      <th>Dataset</th>
      <th>Architecture</th>
      <th>Pass Rate</th>
      <th>Tasks</th>
      <th>Timeout</th>
      <th>Avg Duration</th>
      <th>View</th>
    </tr>
  </thead>
  <tbody>
    ${runs
      .slice()
      .reverse()
      .map((r) => {
        const viewerUrl = `viewer.html?run=${encodeURIComponent(r.runId)}`
        const passed = Math.round(r.passRate * r.total)
        const archLabel =
          r.agentType === 'orchestrator-executor'
            ? 'Orch-Exec'
            : r.agentType === 'single'
              ? 'Tool Loop'
              : r.agentType === 'gemini-computer-use'
                ? 'Gemini CU'
                : r.agentType || '—'
        return `<tr data-config="${escHtml(r.runId)}" data-search="${escHtml(`${r.date} ${r.runId} ${r.model} ${r.dataset} ${archLabel}`)}">
      <td>${escHtml(r.date)}</td>
      <td class="mono">${escHtml(r.runId)}</td>
      <td class="mono" style="max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title="${escHtml(r.model)}">${escHtml(r.model)}</td>
      <td>${escHtml(r.dataset)}</td>
      <td>${escHtml(archLabel)}</td>
      <td class="${r.passRate >= 0.7 ? 'pass' : r.passRate >= 0.4 ? 'neutral' : 'fail'}">${(r.passRate * 100).toFixed(1)}% <span style="color:#6e7681;font-size:11px;">(${passed}/${r.total})</span></td>
      <td>${r.total}</td>
      <td class="${r.timeout > 0 ? 'neutral' : ''}">${r.timeout}</td>
      <td>${(r.avgDurationMs / 1000).toFixed(0)}s</td>
      <td><a href="${viewerUrl}" class="view-link">View &rarr;</a></td>
    </tr>`
      })
      .join('\n')}
  </tbody>
</table>

<script>
(function() {
  'use strict';

  var allRuns = ${JSON.stringify(runs)};
  var configSelect = document.getElementById('config-select');
  var canvas = document.getElementById('chart');
  var ctx = canvas.getContext('2d');
  var tooltip = document.getElementById('tooltip');
  var dpr = window.devicePixelRatio || 1;
  var dotPositions = [];

  function getFilteredRuns() {
    var cfg = configSelect.value;
    return allRuns.filter(function(r) { return r.configName === cfg; });
  }

  function updateDashboard() {
    var runs = getFilteredRuns();
    renderConfigDetails(runs);
    renderStatCards(runs);
    drawChart(runs);
  }

  // Config details card
  function renderConfigDetails(runs) {
    var el = document.getElementById('config-details');
    if (runs.length === 0) { el.innerHTML = '<span style="color:#6e7681;">No runs found for this config.</span>'; return; }
    var latest = runs[runs.length - 1];
    var archLabel = latest.agentType === 'orchestrator-executor' ? 'Orchestrator-Executor'
      : latest.agentType === 'single' ? 'Single Agent (Tool Loop)'
      : latest.agentType === 'gemini-computer-use' ? 'Gemini Computer Use'
      : latest.agentType || 'Unknown';
    el.innerHTML =
      '<div class="config-detail"><span class="cd-label">Architecture</span><span class="cd-value">' + archLabel + '</span></div>' +
      '<div class="config-detail"><span class="cd-label">Model</span><span class="cd-value">' + (latest.model || 'unknown') + '</span></div>' +
      '<div class="config-detail"><span class="cd-label">Dataset</span><span class="cd-value">' + (latest.dataset || 'unknown') + '</span></div>' +
      '<div class="config-detail"><span class="cd-label">Tasks</span><span class="cd-value">' + latest.total + '</span></div>' +
      '<div class="config-detail"><span class="cd-label">Runs</span><span class="cd-value">' + runs.length + '</span></div>';
  }

  // Stat cards
  function renderStatCards(runs) {
    var el = document.getElementById('stat-cards');
    if (runs.length === 0) { el.innerHTML = ''; return; }
    var latest = runs[runs.length - 1];
    var prev = runs.length >= 2 ? runs[runs.length - 2] : null;
    var best = Math.max.apply(null, runs.map(function(r) { return r.passRate; }));
    var delta = prev ? latest.passRate - prev.passRate : 0;
    var sign = delta > 0 ? '+' : '';
    var trendCls = delta > 0 ? 'trend-up' : delta < 0 ? 'trend-down' : 'trend-flat';

    el.innerHTML =
      '<div class="stat-card"><div class="stat-label">Latest Pass Rate</div><div class="stat-value big ' + (latest.passRate >= 0.7 ? 'pass' : 'fail') + '">' + (latest.passRate * 100).toFixed(1) + '%</div></div>' +
      '<div class="stat-card"><div class="stat-label">Trend</div><div class="stat-value ' + trendCls + '">' + (prev ? sign + (delta * 100).toFixed(1) + ' pp' : 'N/A') + '</div></div>' +
      '<div class="stat-card"><div class="stat-label">Best Score</div><div class="stat-value pass">' + (best * 100).toFixed(1) + '%</div></div>' +
      '<div class="stat-card"><div class="stat-label">Avg Duration</div><div class="stat-value">' + (latest.avgDurationMs / 1000).toFixed(0) + 's</div></div>' +
      '<div class="stat-card"><div class="stat-label">Runs</div><div class="stat-value">' + runs.length + '</div></div>';
  }

  // Chart
  function drawChart(runs) {
    var rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    var W = rect.width, H = rect.height;
    var pad = { top: 20, right: 20, bottom: 50, left: 50 };
    var plotW = W - pad.left - pad.right;
    var plotH = H - pad.top - pad.bottom;
    dotPositions = [];

    ctx.clearRect(0, 0, W, H);

    if (runs.length === 0) {
      ctx.fillStyle = '#8b949e';
      ctx.font = '14px sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText('No data for this config', W / 2, H / 2);
      return;
    }

    var scores = runs.map(function(r) { return r.passRate * 100; });
    var minY = Math.max(0, Math.floor(Math.min.apply(null, scores) / 10) * 10 - 10);
    var maxY = Math.min(100, Math.ceil(Math.max.apply(null, scores) / 10) * 10 + 10);
    if (minY === maxY) { minY = Math.max(0, minY - 10); maxY = Math.min(100, maxY + 10); }

    // Grid
    ctx.strokeStyle = '#21262d';
    ctx.lineWidth = 1;
    for (var y = minY; y <= maxY; y += 10) {
      var py = pad.top + plotH - ((y - minY) / (maxY - minY)) * plotH;
      ctx.beginPath(); ctx.moveTo(pad.left, py); ctx.lineTo(pad.left + plotW, py); ctx.stroke();
      ctx.fillStyle = '#8b949e'; ctx.font = '11px sans-serif'; ctx.textAlign = 'right';
      ctx.fillText(y + '%', pad.left - 8, py + 4);
    }

    // X labels
    ctx.fillStyle = '#8b949e'; ctx.font = '11px sans-serif'; ctx.textAlign = 'center';
    runs.forEach(function(r, i) {
      var px = pad.left + (runs.length === 1 ? plotW / 2 : (i / (runs.length - 1)) * plotW);
      ctx.save(); ctx.translate(px, pad.top + plotH + 15); ctx.rotate(-Math.PI / 6);
      ctx.fillText(r.date, 0, 0); ctx.restore();
    });

    // Line
    ctx.strokeStyle = '#58a6ff'; ctx.lineWidth = 2; ctx.beginPath();
    runs.forEach(function(r, i) {
      var px = pad.left + (runs.length === 1 ? plotW / 2 : (i / (runs.length - 1)) * plotW);
      var py2 = pad.top + plotH - ((r.passRate * 100 - minY) / (maxY - minY)) * plotH;
      if (i === 0) ctx.moveTo(px, py2); else ctx.lineTo(px, py2);
    });
    ctx.stroke();

    // Dots
    runs.forEach(function(r, i) {
      var px = pad.left + (runs.length === 1 ? plotW / 2 : (i / (runs.length - 1)) * plotW);
      var py2 = pad.top + plotH - ((r.passRate * 100 - minY) / (maxY - minY)) * plotH;
      dotPositions.push({ x: px, y: py2, run: r });
      ctx.beginPath(); ctx.arc(px, py2, 4, 0, Math.PI * 2);
      ctx.fillStyle = r.passRate >= 0.7 ? '#3fb950' : '#f85149';
      ctx.fill(); ctx.strokeStyle = '#0d1117'; ctx.lineWidth = 2; ctx.stroke();
    });
  }

  // Tooltip
  canvas.addEventListener('mousemove', function(e) {
    var rect = canvas.getBoundingClientRect();
    var mx = e.clientX - rect.left, my = e.clientY - rect.top;
    var closest = null, closestDist = Infinity;
    dotPositions.forEach(function(dot) {
      var d = Math.sqrt(Math.pow(mx - dot.x, 2) + Math.pow(my - dot.y, 2));
      if (d < closestDist) { closestDist = d; closest = dot; }
    });

    if (closest && closestDist < 40) {
      var r = closest.run;
      var passed = Math.round(r.passRate * r.total);
      document.getElementById('tt-date').textContent = r.date;
      document.getElementById('tt-score').textContent = (r.passRate * 100).toFixed(1) + '%';
      document.getElementById('tt-score').style.color = r.passRate >= 0.7 ? '#3fb950' : '#f85149';
      document.getElementById('tt-detail').textContent = passed + '/' + r.total + ' pass \\u00B7 ' + (r.avgDurationMs / 1000).toFixed(0) + 's avg \\u00B7 ' + r.model;
      tooltip.style.display = 'block';

      var tx = closest.x + 12, ty = closest.y - 50;
      if (tx + 200 > rect.width) tx = closest.x - 210;
      if (ty < 0) ty = closest.y + 12;
      tooltip.style.left = tx + 'px'; tooltip.style.top = ty + 'px';

      // Highlight dot
      drawChart(getFilteredRuns());
      ctx.beginPath(); ctx.arc(closest.x, closest.y, 7, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(88, 166, 255, 0.3)'; ctx.fill();
      ctx.beginPath(); ctx.arc(closest.x, closest.y, 5, 0, Math.PI * 2);
      ctx.fillStyle = r.passRate >= 0.7 ? '#3fb950' : '#f85149'; ctx.fill();
      ctx.strokeStyle = '#e6edf3'; ctx.lineWidth = 2; ctx.stroke();
      canvas.style.cursor = 'pointer';
    } else {
      tooltip.style.display = 'none';
      canvas.style.cursor = 'default';
    }
  });

  canvas.addEventListener('mouseleave', function() {
    tooltip.style.display = 'none';
    drawChart(getFilteredRuns());
  });

  canvas.addEventListener('click', function(e) {
    var rect = canvas.getBoundingClientRect();
    var mx = e.clientX - rect.left, my = e.clientY - rect.top;
    dotPositions.forEach(function(dot) {
      if (Math.sqrt(Math.pow(mx - dot.x, 2) + Math.pow(my - dot.y, 2)) < 20) {
        window.open('viewer.html?run=' + encodeURIComponent(dot.run.runId), '_blank');
      }
    });
  });

  // Config selector change
  configSelect.addEventListener('change', function() {
    tooltip.style.display = 'none';
    updateDashboard();
  });

  // Table search
  document.getElementById('table-search').addEventListener('input', function(e) {
    var q = e.target.value.toLowerCase();
    var rows = document.querySelectorAll('#runs-table tbody tr');
    rows.forEach(function(row) {
      var searchText = row.getAttribute('data-search') || '';
      row.classList.toggle('hidden', q && searchText.toLowerCase().indexOf(q) === -1);
    });
  });

  // Resize
  window.addEventListener('resize', function() { tooltip.style.display = 'none'; drawChart(getFilteredRuns()); });

  // Init
  updateDashboard();
})();
</script>

</body>
</html>`

// Step 5: Save locally and upload to R2
const localPath = process.argv[2] || '/tmp/eval-report.html'
await writeFile(localPath, html)
console.log(`Report saved locally: ${localPath}`)

await client.send(
  new PutObjectCommand({
    Bucket: bucket,
    Key: 'report.html',
    Body: html,
    ContentType: 'text/html',
    CacheControl: 'public, max-age=300',
  }),
)

const cdnBaseUrl = (
  process.env.EVAL_R2_CDN_BASE_URL || 'https://eval.browseros.com'
).replace(/\/+$/, '')

console.log(`Report uploaded to R2: ${bucket}/report.html`)
console.log(`  View at: ${cdnBaseUrl}/report.html`)

// Print summary
console.log('\nScore trend:')
for (const run of runs.slice(-10)) {
  const bar = '\u2588'.repeat(Math.round(run.passRate * 20))
  const pct = (run.passRate * 100).toFixed(0).padStart(3)
  console.log(`  ${run.date}  ${pct}% ${bar}`)
}
