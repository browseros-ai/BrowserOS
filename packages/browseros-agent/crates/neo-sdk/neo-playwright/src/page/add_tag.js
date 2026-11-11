/*
 * Insertion algorithms adapted from Playwright v1.64.1, server/frames.ts.
 * Copyright (c) Microsoft Corporation. Licensed under Apache-2.0.
 * Neo passes renderer inputs together and binds this call to the owning frame's document.
 */
// biome-ignore lint/correctness/noUnusedVariables: Rust sends this declaration through Runtime.callFunctionOn.
async function addTag(params) {
  const { kind, url, content, type } = params
  if (kind === 'script') {
    const script = this.createElement('script')
    if (url !== null) {
      script.src = url
      if (type) script.type = type
      const loaded = new Promise((resolve, reject) => {
        script.onload = resolve
        script.onerror = (e) =>
          reject(
            typeof e === 'string'
              ? new Error(e)
              : new Error(`Failed to load script at ${script.src}`),
          )
      })
      this.head.appendChild(script)
      await loaded
    } else {
      script.type = type || 'text/javascript'
      script.text = content
      let error = null
      script.onerror = (e) => (error = e)
      this.head.appendChild(script)
      if (error) throw error
    }
    return script
  }
  const tag = this.createElement(url !== null ? 'link' : 'style')
  if (url !== null) {
    tag.rel = 'stylesheet'
    tag.href = url
  } else {
    tag.type = 'text/css'
    tag.appendChild(this.createTextNode(content))
  }
  const loaded = new Promise((resolve, reject) => {
    tag.onload = resolve
    tag.onerror = reject
  })
  this.head.appendChild(tag)
  await loaded
  return tag
}
