// Shared by the viewer and the trace UI: calls to the gateway, and a small DOM builder.
// Every call goes to the page's own origin; the gateway adds claw-sdk's bearer token.

/** A refusal in the contract's `Error` shape ({code, message}), from claw-sdk or the gateway. */
export class ApiError extends Error {
  constructor(status, code, message) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

/** JSON in, JSON out; a non-2xx answer throws an ApiError with claw-sdk's code and message. */
export async function api(path, { method = "GET", body } = {}) {
  let res;
  try {
    res = await fetch(path, {
      method,
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (error) {
    throw new ApiError(0, "gateway_unreachable", `the gateway did not answer: ${error.message}`);
  }
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    // Not JSON; the text itself is the message below.
  }
  if (!res.ok) {
    throw new ApiError(res.status, data?.code ?? `http_${res.status}`, data?.message ?? text.slice(0, 300));
  }
  return data;
}

/**
 * h("div", {class: "x", onclick}, child, [children], null) builds an element. Strings become
 * text nodes, null/false/undefined are skipped, arrays are flattened; `on*` props are listeners.
 */
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "class") el.className = value;
    else if (key in el && typeof value !== "string") el[key] = value;
    else el.setAttribute(key, value === true ? "" : value);
  }
  el.append(...flat(children));
  return el;
}

function flat(children) {
  const out = [];
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) out.push(...flat(child));
    else out.push(child instanceof Node ? child : String(child));
  }
  return out;
}

export function clock(ms) {
  if (!ms) return "";
  const d = new Date(ms);
  const pad = (n, w = 2) => String(n).padStart(w, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export function duration(ms) {
  if (ms === null || ms === undefined) return "";
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(ms < 10000 ? 2 : 1)} s`;
  return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
}
