/*
 * Playwright v1.64.1 (cfe490d941f992655948eee8fd4fd94e1c4234bf), packages/isomorphic ariaSnapshotRenderer with the yaml and
 * stringUtils it imports, for neo-snapshot's tests: one script for QuickJS that sets the global
 * renderAriaSnapshotAsYaml.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/neo-sdk/neo-playwright/third_party/playwright, by
 * crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
(function () {
'use strict';
// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/stringUtils.ts
function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
function longestCommonSubstring(s1, s2) {
  const n = s1.length;
  const m = s2.length;
  let maxLen = 0;
  let endingIndex = 0;
  const dp = Array(n + 1).fill(null).map(() => Array(m + 1).fill(0));
  for (let i = 1;i <= n; i++) {
    for (let j = 1;j <= m; j++) {
      if (s1[i - 1] === s2[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
        if (dp[i][j] > maxLen) {
          maxLen = dp[i][j];
          endingIndex = i;
        }
      }
    }
  }
  return s1.slice(endingIndex - maxLen, endingIndex);
}
var ansiRegex = new RegExp("([\\u001B\\u009B][[\\]()#?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]*)*)?\\u0007)|(?:(?:\\d{0,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-ntqry=><~])))", "g");

// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/yaml.ts
function yamlEscapeKeyIfNeeded(str) {
  if (!yamlStringNeedsQuotes(str))
    return str;
  return `'` + str.replace(/'/g, `''`) + `'`;
}
function yamlEscapeValueIfNeeded(str) {
  if (!yamlStringNeedsQuotes(str))
    return str;
  return '"' + str.replace(/[\\"\x00-\x1f\x7f-\x9f]/g, (c) => {
    switch (c) {
      case "\\":
        return "\\\\";
      case '"':
        return "\\\"";
      case "\b":
        return "\\b";
      case "\f":
        return "\\f";
      case `
`:
        return "\\n";
      case "\r":
        return "\\r";
      case "\t":
        return "\\t";
      default:
        const code = c.charCodeAt(0);
        return "\\x" + code.toString(16).padStart(2, "0");
    }
  }) + '"';
}
function yamlStringNeedsQuotes(str) {
  if (str.length === 0)
    return true;
  if (/^\s|\s$/.test(str))
    return true;
  if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(str))
    return true;
  if (/^-/.test(str))
    return true;
  if (/[\n:](\s|$)/.test(str))
    return true;
  if (/\s#/.test(str))
    return true;
  if (/[\n\r]/.test(str))
    return true;
  if (/^[&*\],?!>|@"'#%]/.test(str))
    return true;
  if (/[{}`]/.test(str))
    return true;
  if (/^\[/.test(str))
    return true;
  if (!isNaN(Number(str)) || ["y", "n", "yes", "no", "true", "false", "on", "off", "null"].includes(str.toLowerCase()))
    return true;
  return false;
}

// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts
function renderAriaSnapshotAsYaml(snapshot, options = {}) {
  const lines = [];
  const includeText = options.convertStringsToRegex ? textContributesInfo : () => true;
  const renderString = options.convertStringsToRegex ? convertToBestGuessRegex : (str) => str;
  const visitText = (text, depth) => {
    const escaped = yamlEscapeValueIfNeeded(renderString(text));
    if (escaped)
      lines.push(indent(depth) + "- text: " + escaped);
  };
  const createKey = (node) => {
    let key = node.role;
    if (node.name && node.name.length <= 900) {
      const name = renderString(node.name);
      if (name) {
        const isRegex = options.convertStringsToRegex && name !== node.name;
        key += " " + (isRegex ? name : JSON.stringify(name));
      }
    }
    if (node.checked === "mixed")
      key += ` [checked=mixed]`;
    if (node.checked === true)
      key += ` [checked]`;
    if (node.disabled)
      key += ` [disabled]`;
    if (node.expanded)
      key += ` [expanded]`;
    if (node.active)
      key += ` [active]`;
    if (node.invalid === "grammar" || node.invalid === "spelling")
      key += ` [invalid=${node.invalid}]`;
    if (node.invalid === true)
      key += ` [invalid]`;
    if (node.level)
      key += ` [level=${node.level}]`;
    if (node.pressed === "mixed")
      key += ` [pressed=mixed]`;
    if (node.pressed === true)
      key += ` [pressed]`;
    if (node.selected === true)
      key += ` [selected]`;
    if (node.ariaHidden)
      key += ` [aria-hidden]`;
    if (node.ref) {
      key += ` [ref=${node.ref}]`;
      if (node.cursor === "pointer")
        key += " [cursor=pointer]";
    }
    if (node.box)
      key += ` [box=${node.box.x},${node.box.y},${node.box.width},${node.box.height}]`;
    return key;
  };
  const visit = (node, depth) => {
    if (node.role === "text") {
      visitText(node.text || "", depth);
      return;
    }
    options.lineToNode?.set(lines.length, node);
    const escapedKey = indent(depth) + "- " + yamlEscapeKeyIfNeeded(createKey(node));
    const props = [];
    if (node.url !== undefined)
      props.push(["url", node.url]);
    if (node.placeholder !== undefined)
      props.push(["placeholder", node.placeholder]);
    if (node.text === undefined && !props.length && !node.children?.length) {
      lines.push(escapedKey);
    } else if (node.text !== undefined && !props.length) {
      if (includeText(node, node.text))
        lines.push(escapedKey + ": " + yamlEscapeValueIfNeeded(renderString(node.text)));
      else
        lines.push(escapedKey);
    } else {
      lines.push(escapedKey + ":");
      for (const [name, value] of props)
        lines.push(indent(depth + 1) + "- /" + name + ": " + yamlEscapeValueIfNeeded(value));
      if (node.text !== undefined) {
        visitText(includeText(node, node.text) ? node.text : "", depth + 1);
      } else {
        for (const child of node.children || []) {
          if (typeof child === "string")
            visitText(includeText(node, child) ? child : "", depth + 1);
          else
            visit(child, depth + 1);
        }
      }
    }
  };
  for (const node of snapshot)
    visit(node, 0);
  return lines.join(`
`);
}
function indent(depth) {
  return "  ".repeat(depth);
}
function convertToBestGuessRegex(text) {
  const dynamicContent = [
    { regex: /\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/, replacement: "[0-9a-fA-F-]+" },
    { regex: /\b[\d,.]+[bkmBKM]+\b/, replacement: "[\\d,.]+[bkmBKM]+" },
    { regex: /\b\d+[hmsp]+\b/, replacement: "\\d+[hmsp]+" },
    { regex: /\b[\d,.]+[hmsp]+\b/, replacement: "[\\d,.]+[hmsp]+" },
    { regex: /\b\d+,\d+\b/, replacement: "\\d+,\\d+" },
    { regex: /\b\d+\.\d{2,}\b/, replacement: "\\d+\\.\\d+" },
    { regex: /\b\d{2,}\.\d+\b/, replacement: "\\d+\\.\\d+" },
    { regex: /\b\d{2,}\b/, replacement: "\\d+" }
  ];
  let pattern = "";
  let lastIndex = 0;
  const combinedRegex = new RegExp(dynamicContent.map((r) => "(" + r.regex.source + ")").join("|"), "g");
  text.replace(combinedRegex, (match, ...args) => {
    const offset = args[args.length - 2];
    const groups = args.slice(0, -2);
    pattern += escapeRegExp(text.slice(lastIndex, offset));
    for (let i = 0;i < groups.length; i++) {
      if (groups[i]) {
        const { replacement } = dynamicContent[i];
        pattern += replacement;
        break;
      }
    }
    lastIndex = offset + match.length;
    return match;
  });
  if (!pattern)
    return text;
  pattern += escapeRegExp(text.slice(lastIndex));
  return String(new RegExp(pattern));
}
function textContributesInfo(node, text) {
  if (!text.length)
    return false;
  if (!node.name)
    return true;
  const substr = text.length <= 200 && node.name.length <= 200 ? longestCommonSubstring(text, node.name) : "";
  let filtered = text;
  while (substr && filtered.includes(substr))
    filtered = filtered.replace(substr, "");
  return filtered.trim().length / text.length > 0.1;
}
globalThis.renderAriaSnapshotAsYaml = renderAriaSnapshotAsYaml;
})();
