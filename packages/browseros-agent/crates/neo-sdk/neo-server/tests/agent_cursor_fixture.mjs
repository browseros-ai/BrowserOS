// biome-ignore-all lint/correctness/noUnusedVariables: Rust appends the caller to each SDK program.
// Shared by the short SDK programs in agent_cursor_browseros.rs.
const html = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    * { box-sizing:border-box } body { margin:0; background:white }
    #target { position:absolute; left:80px; top:64px; width:80px; height:40px;
      padding:0; border:0; background:#d7ae64; color:black; outline:0; user-select:none }
  </style>
  <button id="target">Target</button><input id="field"><input id="box" type="checkbox">`
const selector = 'browseros-agent-pointer[data-browseros-agent-pointer]'

async function fresh() {
  await page.goto(`data:text/html,${encodeURIComponent(html)}`)
  await observe(false)
}

// Use the top document's clock for both frame and top events.
async function observe(child) {
  await page.evaluate(
    ({ child, selector }) => {
      const doc = child
        ? document.querySelector('iframe').contentDocument
        : document
      const scope = child ? 'frame' : 'top'
      if (!child) window.cursorEvents = []
      const record = (kind, fields = {}) =>
        window.cursorEvents.push({
          t: performance.now(),
          kind,
          scope,
          ...fields,
        })
      const appeared = () => {
        record('overlay')
        window.onCursorAppearance?.()
      }
      if (doc.querySelector(selector)) appeared()
      new MutationObserver((records) => {
        for (const mutation of records)
          for (const node of mutation.addedNodes)
            if (node.matches?.(selector) || node.querySelector?.(selector))
              appeared()
      }).observe(doc, { childList: true, subtree: true })
      for (const kind of [
        'pointerdown',
        'mousedown',
        'click',
        'dblclick',
        'auxclick',
      ]) {
        doc.addEventListener(
          kind,
          (e) =>
            record(kind, {
              id: e.target.id,
              x: e.clientX,
              y: e.clientY,
              button: e.button,
              detail: e.detail,
              shift: e.shiftKey,
              trusted: e.isTrusted,
            }),
          true,
        )
      }
      doc.addEventListener('contextmenu', (e) => e.preventDefault())
    },
    { child, selector },
  )
}

const events = () => page.evaluate(() => window.cursorEvents)
function check(ok, message, seen) {
  if (!ok) throw new Error(`${message}: ${JSON.stringify(seen)}`)
}

async function landing(
  action,
  {
    on,
    x = 120,
    y = 84,
    count = 1,
    right = false,
    shift = false,
    scope = 'top',
  },
) {
  const start = Date.now()
  try {
    await action()
  } catch (error) {
    throw new Error(
      `${error.message}; page events: ${JSON.stringify(await events())}`,
    )
  }
  const wallMs = Date.now() - start
  const log = await events()
  const overlays = log.filter((e) => e.kind === 'overlay')
  const input = log.filter((e) =>
    ['pointerdown', 'mousedown', 'click', 'dblclick', 'auxclick'].includes(
      e.kind,
    ),
  )
  const expected = []
  for (let n = 1; n <= count; ++n) {
    expected.push(
      ['pointerdown', 0],
      ['mousedown', n],
      [right ? 'auxclick' : 'click', n],
    )
    if (n === 2 && !right) expected.push(['dblclick', n])
  }
  check(
    input.length === expected.length &&
      input.every(
        (e, i) =>
          e.kind === expected[i][0] &&
          e.detail === expected[i][1] &&
          e.trusted &&
          e.scope === scope &&
          e.id === 'target' &&
          Math.abs(e.x - x) <= 1 &&
          Math.abs(e.y - y) <= 1 &&
          e.button === (right ? 2 : 0) &&
          e.shift === shift,
      ),
    'input sequence or point',
    { wallMs, log },
  )
  check(
    overlays.length === (on ? 1 : 0) &&
      overlays.every((e) => e.scope === 'top'),
    'overlay count or document',
    { wallMs, log },
  )
  const gapMs = on
    ? input.find((e) => e.kind === 'mousedown').t - overlays[0].t
    : null
  if (on)
    check(
      overlays[0].t < input[0].t && gapMs >= 250,
      'too little glide before input',
      { wallMs, log },
    )
  return { wallMs, gapMs, log }
}

async function matrix(name, on) {
  await fresh()
  const target = page.locator('#target')
  switch (name) {
    case 'locator':
      return landing(() => target.click(), { on })
    case 'ref': {
      const tree = await page.ariaSnapshot()
      const ref = /button "Target" \[ref=((?:f\d+)?e\d+)\]/.exec(tree)?.[1]
      check(ref, 'missing button ref', tree)
      return landing(() => page.getByRef(ref).click(), { on })
    }
    case 'locator.dblclick':
      return landing(() => target.dblclick(), { on, count: 2 })
    case 'mouse.click':
      return landing(() => page.mouse.click(112, 77), { on, x: 112, y: 77 })
    case 'mouse.dblclick':
      return landing(() => page.mouse.dblclick(108, 81), {
        on,
        x: 108,
        y: 81,
        count: 2,
      })
    case 'locator.options':
      return landing(
        () =>
          target.click({
            button: 'right',
            modifiers: ['Shift'],
            position: { x: 9, y: 6 },
          }),
        { on, x: 89, y: 70, right: true, shift: true },
      )
    case 'mouse.clickCount':
      return landing(() => page.mouse.click(130, 90, { clickCount: 3 }), {
        on,
        x: 130,
        y: 90,
        count: 3,
      })
    default:
      throw new Error(`unknown case ${name}`)
  }
}

async function withoutCursor(name) {
  await fresh()
  // Record the move even if a previous document left the physical mouse at the same point.
  await page.evaluate(() =>
    document.addEventListener(
      'pointermove',
      (e) =>
        window.cursorEvents.push({
          t: performance.now(),
          kind: 'move',
          id: e.target.id,
          x: e.clientX,
          y: e.clientY,
        }),
      true,
    ),
  )
  const target = page.locator('#target')
  switch (name) {
    case 'trial':
      await target.click({ trial: true })
      break
    case 'hover':
      await target.hover()
      break
    case 'fill':
      await page.locator('#field').fill('typed')
      break
    case 'check':
      await page.locator('#box').check()
      break
    case 'move':
      await page.mouse.move(101, 72)
      break
    case 'missing':
    case 'covered': {
      if (name === 'covered')
        await page.evaluate(() => {
          const cover = document.createElement('div')
          cover.style.cssText = 'position:fixed;inset:0;background:white'
          document.body.append(cover)
        })
      let error
      try {
        await (name === 'covered' ? target : page.locator('#missing')).click({
          // Give an incorrectly admitted covered click enough time to draw a cursor.
          timeout: name === 'covered' ? 3000 : 800,
        })
      } catch (e) {
        error = e
      }
      check(
        error?.name === 'TimeoutError' || /timeout/i.test(error?.message),
        'actionability refusal',
        String(error),
      )
      break
    }
    default:
      throw new Error(`unknown case ${name}`)
  }
  const seen = await page.evaluate(() => ({
    log: window.cursorEvents,
    field: document.querySelector('#field').value,
    checked: document.querySelector('#box').checked,
  }))
  const input = seen.log.filter((e) =>
    ['pointerdown', 'mousedown', 'click', 'dblclick', 'auxclick'].includes(
      e.kind,
    ),
  )
  check(!seen.log.some((e) => e.kind === 'overlay'), 'unexpected overlay', seen)
  check(
    name === 'check'
      ? input.map((e) => e.kind).join() === 'pointerdown,mousedown,click' &&
          input.every((e) => e.id === 'box' && e.trusted) &&
          seen.checked
      : input.length === 0,
    'unexpected button press',
    seen,
  )
  if (name === 'fill') check(seen.field === 'typed', 'field value', seen)
  if (name === 'hover' || name === 'move')
    check(
      seen.log.some(
        (e) =>
          e.kind === 'move' &&
          e.id === 'target' &&
          (name !== 'move' || (e.x === 101 && e.y === 72)),
      ),
      'move missed',
      seen,
    )
  return seen
}

async function movingTarget() {
  await fresh()
  await page.evaluate(() => {
    window.onCursorAppearance = () =>
      setTimeout(() => {
        document.querySelector('#target').style.left = '230px'
        window.cursorEvents.push({ kind: 'moved', t: performance.now() })
      }, 100)
  })
  const hit = await landing(() => page.locator('#target').click(), {
    on: true,
    x: 270,
  })
  const moved = hit.log.find((e) => e.kind === 'moved')
  check(
    moved &&
      moved.t > hit.log[0].t &&
      moved.t < hit.log.find((e) => e.kind === 'mousedown').t,
    'button must move during the glide',
    hit,
  )
  return hit
}

async function framed(transform) {
  await fresh()
  await page.evaluate(
    ({ html, transform }) =>
      new Promise((resolve) => {
        const f = document.createElement('iframe')
        f.style.cssText =
          'position:absolute;left:20px;top:25px;border:0;width:240px;height:130px;transform-origin:0 0;transform:' +
          transform
        f.onload = () => resolve(true)
        f.srcdoc = html
        document.body.appendChild(f)
      }),
    { html, transform },
  )
  await observe(true)
  return landing(() => page.frameLocator('iframe').locator('#target').click(), {
    on: true,
    scope: 'frame',
  })
}

// BrowserOS can leave the emulated finger down until the locator times out. Even then the
// page must receive exactly one press at the CSS point and the cursor must have glided there.
async function phonePress(on) {
  const metrics = await page.evaluate(() => [
    innerWidth,
    innerHeight,
    devicePixelRatio,
    navigator.maxTouchPoints,
  ])
  check(
    metrics[0] === 366 &&
      metrics[1] === 700 &&
      metrics[2] === 2 &&
      metrics[3] > 0,
    'phone metrics',
    metrics,
  )
  let error = null
  try {
    // Leave enough time for the glide and capture before the four-second idle removal.
    await page.locator('#target').click({ timeout: 3000 })
  } catch (e) {
    error = {
      name: e.name,
      code: e.code,
      delivery: e.delivery,
      message: e.message,
    }
  }
  const log = await events()
  const presses = log.filter((e) => e.kind === 'pointerdown')
  const overlays = log.filter((e) => e.kind === 'overlay')
  const seen = { metrics, error, log }
  check(
    presses.length === 1 &&
      presses.every(
        (e) =>
          e.id === 'target' &&
          e.scope === 'top' &&
          e.trusted &&
          Math.abs(e.x - 120) <= 1 &&
          Math.abs(e.y - 84) <= 1,
      ),
    'phone press',
    seen,
  )
  check(
    overlays.length === (on ? 1 : 0) &&
      overlays.every((e) => e.scope === 'top' && presses[0].t - e.t >= 250),
    'phone cursor before press',
    seen,
  )
  if (error)
    check(
      error.name === 'TimeoutError' &&
        error.delivery === 'unknown' &&
        error.message.includes('performing click action'),
      'unexpected phone failure',
      seen,
    )
  else
    check(
      log.filter((e) => e.kind === 'click' && e.id === 'target' && e.trusted)
        .length === 1,
      'completed phone click',
      seen,
    )
  return seen
}

// Measure the solid button face in the actual PNG. Its center is the CSS landing point
// regardless of the framebuffer size, device pixel ratio, and preview zoom.
async function targetPixels(png) {
  return page.evaluate(async (data) => {
    const image = new Image()
    image.src = `data:image/png;base64,${data}`
    await image.decode()
    const canvas = document.createElement('canvas')
    canvas.width = image.width
    canvas.height = image.height
    const ctx = canvas.getContext('2d')
    ctx.drawImage(image, 0, 0)
    const { width, height } = canvas
    const pixels = ctx.getImageData(0, 0, width, height).data
    let left = width,
      right = -1,
      top = height,
      bottom = -1
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        const i = (y * width + x) * 4
        if (
          pixels[i] === 215 &&
          pixels[i + 1] === 174 &&
          pixels[i + 2] === 100
        ) {
          left = Math.min(left, x)
          right = Math.max(right, x)
          top = Math.min(top, y)
          bottom = Math.max(bottom, y)
        }
      }
    if (right < left) throw new Error('button face absent from native capture')
    const scale = (right - left + 1) / 80
    const cx = (left + right) / 2,
      cy = (top + bottom) / 2
    let blue = 0
    for (let y = Math.ceil(cy - 8 * scale); y <= cy + 8 * scale; y++)
      for (let x = Math.ceil(cx - 8 * scale); x <= cx + 8 * scale; x++) {
        const i = (y * width + x) * 4
        if (pixels[i] < 50 && pixels[i + 1] < 130 && pixels[i + 2] > 140) blue++
      }
    return { width, height, button: [left, top, right, bottom], blue }
  }, png)
}
