/**
 * Neo: Zuzu-style password swap, submit approval and exact-string read redaction.
 * Supply trusted meta: { origin: 'https://your-login-site.example', ownerApproved: false }.
 * This example is not a complete policy for hostile pages or arbitrary agent JavaScript.
 */
/// <reference path="../../../neo-quickjs-injected/hooks.d.ts" />

/** @param {import('../../../neo-quickjs-injected/hooks').HookContext} ctx */
function configuration(ctx) {
  const meta = ctx.meta
  if (
    !meta ||
    typeof meta !== 'object' ||
    !('origin' in meta) ||
    typeof meta.origin !== 'string' ||
    new URL(meta.origin).origin !== meta.origin
  ) {
    throw new Error('The host must supply an exact login origin')
  }
  return {
    origin: meta.origin,
    ownerApproved: 'ownerApproved' in meta && meta.ownerApproved === true,
  }
}

neo.on('beforeRun', (_event, ctx) => {
  configuration(ctx)
  ctx.state.secrets = []
})

neo.on('beforeAction', { kinds: ['fill'] }, async (event, ctx) => {
  const password = event.args.value === '{{password}}'
  if (!password && event.args.value !== '{{username}}') return
  const { origin } = configuration(ctx)
  if (
    !event.element ||
    !event.page ||
    new URL(event.page.url()).origin !== origin
  ) {
    return { block: 'Credentials require the expected site and a pinned input' }
  }
  const box = await event.element.evaluate((node) => ({
    input: node instanceof HTMLInputElement,
    type: node instanceof HTMLInputElement ? node.type : '',
    origin: node.ownerDocument?.location.origin,
  }))
  if (
    !box.input ||
    box.origin !== origin ||
    (password ? box.type !== 'password' : !['text', 'email'].includes(box.type))
  ) {
    return { block: 'Credentials require the expected input type and origin' }
  }
  const login = await ctx.host('vault.login', { origin })
  if (!login || typeof login !== 'object') {
    throw new Error('The vault returned no credential')
  }
  const value = password
    ? 'password' in login
      ? login.password
      : undefined
    : 'username' in login
      ? login.username
      : undefined
  if (typeof value !== 'string' || !value.length)
    throw new Error('The vault returned an invalid credential')
  const secrets = ctx.state.secrets
  if (!Array.isArray(secrets))
    throw new Error('The credential ledger is missing')
  if (!secrets.includes(value)) secrets.push(value)
  // Keep selector/source/target and all other wire arguments unchanged.
  return { replaceArgs: { ...event.args, value } }
})

neo.on('beforeAction', { groups: ['pointer', 'type'] }, async (event, ctx) => {
  if (configuration(ctx).ownerApproved) return
  const enter =
    typeof event.args.key === 'string' &&
    event.args.key.split('+').includes('Enter')
  if (event.group !== 'pointer' && !enter) return
  // Raw mouse/touch/keyboard calls have no pin: this example refuses those paths without approval.
  if (!event.element)
    return {
      block: 'Owner approval required for input without a pinned target',
    }
  const submits = await event.element.evaluate((node, enter) => {
    if (!(node instanceof Element)) return true
    const control = node.closest('button,input,[role="button"],a') || node
    const form =
      control instanceof HTMLButtonElement ||
      control instanceof HTMLInputElement
        ? control.form
        : control.closest('form')
    if (enter && form) return true
    const type =
      control instanceof HTMLButtonElement ||
      control instanceof HTMLInputElement
        ? control.type
        : control.getAttribute('type')
    const text = control.textContent || control.getAttribute('value') || ''
    return (
      type === 'submit' ||
      type === 'image' ||
      /\b(pay|submit|place\s+order)\b/i.test(text)
    )
  }, enter)
  if (submits) return { block: 'Owner approval required to submit or pay' }
})

/**
 * Preserve the wire envelope, object keys, arrays and primitive types. Evaluation replies
 * contain Playwright's serialized value tree; its string leaves need redaction too.
 * @param {unknown} value
 * @param {string[]} secrets
 * @returns {unknown}
 */
function redact(value, secrets) {
  if (typeof value === 'string') {
    let text = value
    for (const secret of secrets) text = text.split(secret).join('•••')
    return text
  }
  if (Array.isArray(value)) return value.map((item) => redact(item, secrets))
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, redact(item, secrets)]),
    )
  }
  return value
}

neo.on('afterAction', { groups: ['read', 'pageScript'] }, (event, ctx) => {
  const secrets = ctx.state.secrets
  // Void/error replies cannot be replaced. Only replace when a string actually changed.
  if (event.error || !Array.isArray(secrets) || !secrets.length) return
  const replacement = redact(event.result, secrets)
  if (JSON.stringify(replacement) !== JSON.stringify(event.result))
    return { replace: replacement }
})
