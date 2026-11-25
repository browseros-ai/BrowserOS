/*
 * Today's snapshot renderer with the Playwright v1.64.1 (cfe490d941f992655948eee8fd4fd94e1c4234bf) packages/isomorphic
 * modules it imports, for neo-snapshot's tests: one script for QuickJS that sets the global
 * todaysAnswer (crates/neo-sdk/neo-playwright-js/scripts/snapshot-cases/answer.ts).
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built from crates/neo-sdk/neo-playwright-js/src/prelude and
 * crates/neo-sdk/neo-playwright/third_party/playwright, by crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Every upstream module is built unmodified. Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
(function () {
'use strict';
// crates/neo-sdk/neo-playwright-js/src/prelude/cover.ts
var ON_TOP = `
  On top: `;
function shapeCovered(error) {
  const cover = Reflect.get(error, "cover");
  if (typeof cover !== "object" || cover === null)
    return;
  const block = Reflect.get(cover, "block");
  if (typeof block === "string" && block !== "")
    error.message = underOnTop(error.message, block);
  const { block: _left, ...kept } = cover;
  Reflect.set(error, "cover", kept);
}
function underOnTop(message, lines) {
  const at = message.indexOf(ON_TOP);
  if (at < 0)
    return message;
  const end = message.indexOf(`
`, at + ON_TOP.length);
  return end < 0 ? `${message}
${lines}` : `${message.slice(0, end)}
${lines}${message.slice(end)}`;
}

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-cap.ts
var CHARS_PER_TOKEN = 4;
function aboutTokens(chars) {
  return Math.max(100, Math.round(chars / CHARS_PER_TOKEN / 100) * 100);
}
function partialHead(result) {
  const roots = result.closedRoots;
  return roots ? [
    `# partial: closed shadow roots not all read (${roots.sent} of ${roots.found})`
  ] : [];
}
function cap(head, pinned, lines, limit, call = "ariaSnapshot()") {
  const whole = [...head, ...pinned, ...lines.map((line) => line.text)].join(`
`);
  const measured = whole.length - pinned.reduce((sum, line) => sum + line.length + 1, 0);
  if (measured <= limit)
    return whole;
  let used = -1;
  let taken = 0;
  for (const text of [...head, ...lines.map((line) => line.text)]) {
    if (used + 1 + text.length > limit)
      break;
    used += 1 + text.length;
    taken += 1;
  }
  const top = head.slice(0, taken);
  let cut = Math.max(0, taken - head.length);
  for (let index = 0;index < cut; index++) {
    const end = lines[index].keptEnd;
    if (end !== undefined && end > cut) {
      cut = index;
      break;
    }
  }
  const blocks = [];
  let more = 0;
  let withRefs = 0;
  let later;
  for (let index = cut;index < lines.length; ) {
    const line = lines[index];
    const end = line.keptEnd;
    if (end !== undefined) {
      const indent = line.text.length - line.text.trimStart().length;
      for (const kept of lines.slice(index, end))
        blocks.push(kept.text.slice(indent));
      index = end;
      continue;
    }
    more += 1;
    if (line.ref)
      withRefs += 1;
    later ??= line.section;
    index += 1;
  }
  if (more === 0)
    return whole;
  const size = `the whole snapshot is about ${aboutTokens(measured)} tokens`;
  const open = openSection(lines, cut, limit);
  const offer = open ? `page.getByRef('${open.ref}').${call} shows all of the ${open.label} section. Or pass a larger maxTokens.` : later ? `page.getByRef('${later.ref}').${call} shows the ${later.label} section, which is not shown above. Or pass a larger maxTokens.` : "Pass a larger maxTokens.";
  const cutLine = `# cut: ${more} more lines (${withRefs} with refs); ${size}. ${offer}`;
  return [
    ...top,
    ...pinned,
    ...linesBefore(lines, cut),
    cutLine,
    ...blocks
  ].join(`
`);
}
function linesBefore(lines, cut) {
  return lines.slice(0, cut).map((line) => line.text);
}
function openSection(lines, cut, limit) {
  for (let index = 0;index < cut; index++) {
    const section = lines[index].section;
    if (section === undefined || section.end <= cut)
      continue;
    const own = lines.slice(index, section.end).map((line) => line.text);
    if (own.join(`
`).length <= limit)
      return section;
  }
  return;
}

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-inert.ts
function inertRoots(json) {
  const roots = [];
  const pending = [...json].reverse();
  while (pending.length) {
    const node = pending.pop();
    if (!node)
      continue;
    if (node.inert)
      roots.push(node);
    else {
      const children = node.children ?? [];
      for (let index = children.length - 1;index >= 0; index--) {
        const child = children[index];
        if (child && typeof child !== "string")
          pending.push(child);
      }
    }
  }
  return roots;
}

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-on-top.ts
function onTopHead(result) {
  return (result.onTop ?? []).flatMap((layer) => layer.line.split(`
`));
}
function isOnTopLine(line) {
  return line.startsWith("# on top: ") || line.startsWith("#   its controls: ") || line.startsWith("# no longer on top: ");
}
function onTopDelta(previous, result) {
  const current = new Set((result.onTop ?? []).map((layer) => layer.ref));
  const removed = [];
  for (const line of previous.split(`
`)) {
    const match = /^# on top: (\S+ (?:"(?:[^"\\]|\\.)*" )?\[ref=([^\]]+)\])/.exec(line);
    if (match && !current.has(match[2]))
      removed.push(`# no longer on top: ${match[1]}`);
  }
  return [...onTopHead(result), ...removed];
}
function onTopContent(result) {
  const layers = (result.onTop ?? []).filter((layer) => layer.wholeView);
  if (!layers.length)
    return { first: [], rest: result.json };
  const front = new Map(layers.map((layer) => [layer.ref, []]));
  const nodes = [];
  const selected = new Set;
  const pending = [...result.json].reverse();
  while (pending.length) {
    const node = pending.pop();
    if (!node)
      continue;
    nodes.push(node);
    if (node.inert)
      continue;
    const block = node.onTop ? front.get(node.onTop) : undefined;
    if (block) {
      selected.add(node);
      block.push(node);
    } else
      for (const child of [...node.children ?? []].reverse())
        if (typeof child !== "string")
          pending.push(child);
  }
  const remaining = new Map;
  for (const node of nodes.reverse()) {
    if (selected.has(node)) {
      remaining.set(node, undefined);
      continue;
    }
    const children = node.children?.flatMap((child) => {
      if (typeof child === "string" || !remaining.has(child))
        return [child];
      const kept = remaining.get(child);
      return kept ? [kept] : [];
    });
    const changed = children && (children.length !== node.children?.length || children.some((child, index) => child !== node.children?.[index]));
    remaining.set(node, changed ? { ...node, children } : node);
  }
  return {
    first: layers.flatMap((layer) => front.get(layer.ref) ?? []),
    rest: result.json.flatMap((node) => {
      const kept = remaining.get(node);
      return kept ? [kept] : [];
    })
  };
}

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-unmarked.ts
var PREFIX = "# refs missing: ";
function unmarkedHead(result) {
  return result.unmarked ? [
    `${PREFIX}the check for clickable elements did not finish on this page (${result.unmarked.reason}). Some clickable elements may have no ref here, and content inside closed shadow roots may be missing. Take the snapshot again; if this line is still here, use a locator.`
  ] : [];
}
function isUnmarkedLine(line) {
  return line.startsWith(PREFIX);
}

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
// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/selectorParser.ts
var kNestedSelectorNames = new Set(["internal:has", "internal:has-not", "internal:and", "internal:or", "internal:chain", "left-of", "right-of", "above", "below", "near"]);
var kNestedSelectorNamesWithDistance = new Set(["left-of", "right-of", "above", "below", "near"]);
var customCSSNames = new Set(["not", "is", "where", "has", "scope", "light", "visible", "text", "text-matches", "text-is", "has-text", "above", "below", "right-of", "left-of", "near", "nth-match"]);
// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/mimeType.ts
var types = new Map([
  ["ez", "application/andrew-inset"],
  ["aw", "application/applixware"],
  ["atom", "application/atom+xml"],
  ["atomcat", "application/atomcat+xml"],
  ["atomdeleted", "application/atomdeleted+xml"],
  ["atomsvc", "application/atomsvc+xml"],
  ["dwd", "application/atsc-dwd+xml"],
  ["held", "application/atsc-held+xml"],
  ["rsat", "application/atsc-rsat+xml"],
  ["bdoc", "application/bdoc"],
  ["xcs", "application/calendar+xml"],
  ["ccxml", "application/ccxml+xml"],
  ["cdfx", "application/cdfx+xml"],
  ["cdmia", "application/cdmi-capability"],
  ["cdmic", "application/cdmi-container"],
  ["cdmid", "application/cdmi-domain"],
  ["cdmio", "application/cdmi-object"],
  ["cdmiq", "application/cdmi-queue"],
  ["cu", "application/cu-seeme"],
  ["mpd", "application/dash+xml"],
  ["davmount", "application/davmount+xml"],
  ["dbk", "application/docbook+xml"],
  ["dssc", "application/dssc+der"],
  ["xdssc", "application/dssc+xml"],
  ["ecma", "application/ecmascript"],
  ["es", "application/ecmascript"],
  ["emma", "application/emma+xml"],
  ["emotionml", "application/emotionml+xml"],
  ["epub", "application/epub+zip"],
  ["exi", "application/exi"],
  ["exp", "application/express"],
  ["fdt", "application/fdt+xml"],
  ["pfr", "application/font-tdpfr"],
  ["geojson", "application/geo+json"],
  ["gml", "application/gml+xml"],
  ["gpx", "application/gpx+xml"],
  ["gxf", "application/gxf"],
  ["gz", "application/gzip"],
  ["hjson", "application/hjson"],
  ["stk", "application/hyperstudio"],
  ["ink", "application/inkml+xml"],
  ["inkml", "application/inkml+xml"],
  ["ipfix", "application/ipfix"],
  ["its", "application/its+xml"],
  ["ear", "application/java-archive"],
  ["jar", "application/java-archive"],
  ["war", "application/java-archive"],
  ["ser", "application/java-serialized-object"],
  ["class", "application/java-vm"],
  ["js", "application/javascript"],
  ["mjs", "application/javascript"],
  ["json", "application/json"],
  ["map", "application/json"],
  ["json5", "application/json5"],
  ["jsonml", "application/jsonml+json"],
  ["jsonld", "application/ld+json"],
  ["lgr", "application/lgr+xml"],
  ["lostxml", "application/lost+xml"],
  ["hqx", "application/mac-binhex40"],
  ["cpt", "application/mac-compactpro"],
  ["mads", "application/mads+xml"],
  ["webmanifest", "application/manifest+json"],
  ["mrc", "application/marc"],
  ["mrcx", "application/marcxml+xml"],
  ["ma", "application/mathematica"],
  ["mb", "application/mathematica"],
  ["nb", "application/mathematica"],
  ["mathml", "application/mathml+xml"],
  ["mbox", "application/mbox"],
  ["mscml", "application/mediaservercontrol+xml"],
  ["metalink", "application/metalink+xml"],
  ["meta4", "application/metalink4+xml"],
  ["mets", "application/mets+xml"],
  ["maei", "application/mmt-aei+xml"],
  ["musd", "application/mmt-usd+xml"],
  ["mods", "application/mods+xml"],
  ["m21", "application/mp21"],
  ["mp21", "application/mp21"],
  ["m4p", "application/mp4"],
  ["mp4s", "application/mp4"],
  ["doc", "application/msword"],
  ["dot", "application/msword"],
  ["mxf", "application/mxf"],
  ["nq", "application/n-quads"],
  ["nt", "application/n-triples"],
  ["cjs", "application/node"],
  ["bin", "application/octet-stream"],
  ["bpk", "application/octet-stream"],
  ["buffer", "application/octet-stream"],
  ["deb", "application/octet-stream"],
  ["deploy", "application/octet-stream"],
  ["dist", "application/octet-stream"],
  ["distz", "application/octet-stream"],
  ["dll", "application/octet-stream"],
  ["dmg", "application/octet-stream"],
  ["dms", "application/octet-stream"],
  ["dump", "application/octet-stream"],
  ["elc", "application/octet-stream"],
  ["exe", "application/octet-stream"],
  ["img", "application/octet-stream"],
  ["iso", "application/octet-stream"],
  ["lrf", "application/octet-stream"],
  ["mar", "application/octet-stream"],
  ["msi", "application/octet-stream"],
  ["msm", "application/octet-stream"],
  ["msp", "application/octet-stream"],
  ["pkg", "application/octet-stream"],
  ["so", "application/octet-stream"],
  ["oda", "application/oda"],
  ["opf", "application/oebps-package+xml"],
  ["ogx", "application/ogg"],
  ["omdoc", "application/omdoc+xml"],
  ["onepkg", "application/onenote"],
  ["onetmp", "application/onenote"],
  ["onetoc", "application/onenote"],
  ["onetoc2", "application/onenote"],
  ["oxps", "application/oxps"],
  ["relo", "application/p2p-overlay+xml"],
  ["xer", "application/patch-ops-error+xml"],
  ["pdf", "application/pdf"],
  ["pgp", "application/pgp-encrypted"],
  ["asc", "application/pgp-signature"],
  ["sig", "application/pgp-signature"],
  ["prf", "application/pics-rules"],
  ["p10", "application/pkcs10"],
  ["p7c", "application/pkcs7-mime"],
  ["p7m", "application/pkcs7-mime"],
  ["p7s", "application/pkcs7-signature"],
  ["p8", "application/pkcs8"],
  ["ac", "application/pkix-attr-cert"],
  ["cer", "application/pkix-cert"],
  ["crl", "application/pkix-crl"],
  ["pkipath", "application/pkix-pkipath"],
  ["pki", "application/pkixcmp"],
  ["pls", "application/pls+xml"],
  ["ai", "application/postscript"],
  ["eps", "application/postscript"],
  ["ps", "application/postscript"],
  ["provx", "application/provenance+xml"],
  ["pskcxml", "application/pskc+xml"],
  ["raml", "application/raml+yaml"],
  ["owl", "application/rdf+xml"],
  ["rdf", "application/rdf+xml"],
  ["rif", "application/reginfo+xml"],
  ["rnc", "application/relax-ng-compact-syntax"],
  ["rl", "application/resource-lists+xml"],
  ["rld", "application/resource-lists-diff+xml"],
  ["rs", "application/rls-services+xml"],
  ["rapd", "application/route-apd+xml"],
  ["sls", "application/route-s-tsid+xml"],
  ["rusd", "application/route-usd+xml"],
  ["gbr", "application/rpki-ghostbusters"],
  ["mft", "application/rpki-manifest"],
  ["roa", "application/rpki-roa"],
  ["rsd", "application/rsd+xml"],
  ["rss", "application/rss+xml"],
  ["rtf", "application/rtf"],
  ["sbml", "application/sbml+xml"],
  ["scq", "application/scvp-cv-request"],
  ["scs", "application/scvp-cv-response"],
  ["spq", "application/scvp-vp-request"],
  ["spp", "application/scvp-vp-response"],
  ["sdp", "application/sdp"],
  ["senmlx", "application/senml+xml"],
  ["sensmlx", "application/sensml+xml"],
  ["setpay", "application/set-payment-initiation"],
  ["setreg", "application/set-registration-initiation"],
  ["shf", "application/shf+xml"],
  ["sieve", "application/sieve"],
  ["siv", "application/sieve"],
  ["smi", "application/smil+xml"],
  ["smil", "application/smil+xml"],
  ["rq", "application/sparql-query"],
  ["srx", "application/sparql-results+xml"],
  ["gram", "application/srgs"],
  ["grxml", "application/srgs+xml"],
  ["sru", "application/sru+xml"],
  ["ssdl", "application/ssdl+xml"],
  ["ssml", "application/ssml+xml"],
  ["swidtag", "application/swid+xml"],
  ["tei", "application/tei+xml"],
  ["teicorpus", "application/tei+xml"],
  ["tfi", "application/thraud+xml"],
  ["tsd", "application/timestamped-data"],
  ["toml", "application/toml"],
  ["trig", "application/trig"],
  ["ttml", "application/ttml+xml"],
  ["ubj", "application/ubjson"],
  ["rsheet", "application/urc-ressheet+xml"],
  ["td", "application/urc-targetdesc+xml"],
  ["vxml", "application/voicexml+xml"],
  ["wasm", "application/wasm"],
  ["wgt", "application/widget"],
  ["hlp", "application/winhlp"],
  ["wsdl", "application/wsdl+xml"],
  ["wspolicy", "application/wspolicy+xml"],
  ["xaml", "application/xaml+xml"],
  ["xav", "application/xcap-att+xml"],
  ["xca", "application/xcap-caps+xml"],
  ["xdf", "application/xcap-diff+xml"],
  ["xel", "application/xcap-el+xml"],
  ["xns", "application/xcap-ns+xml"],
  ["xenc", "application/xenc+xml"],
  ["xht", "application/xhtml+xml"],
  ["xhtml", "application/xhtml+xml"],
  ["xlf", "application/xliff+xml"],
  ["rng", "application/xml"],
  ["xml", "application/xml"],
  ["xsd", "application/xml"],
  ["xsl", "application/xml"],
  ["dtd", "application/xml-dtd"],
  ["xop", "application/xop+xml"],
  ["xpl", "application/xproc+xml"],
  ["*xsl", "application/xslt+xml"],
  ["xslt", "application/xslt+xml"],
  ["xspf", "application/xspf+xml"],
  ["mxml", "application/xv+xml"],
  ["xhvml", "application/xv+xml"],
  ["xvm", "application/xv+xml"],
  ["xvml", "application/xv+xml"],
  ["yang", "application/yang"],
  ["yin", "application/yin+xml"],
  ["zip", "application/zip"],
  ["*3gpp", "audio/3gpp"],
  ["adp", "audio/adpcm"],
  ["amr", "audio/amr"],
  ["au", "audio/basic"],
  ["snd", "audio/basic"],
  ["kar", "audio/midi"],
  ["mid", "audio/midi"],
  ["midi", "audio/midi"],
  ["rmi", "audio/midi"],
  ["mxmf", "audio/mobile-xmf"],
  ["*mp3", "audio/mp3"],
  ["m4a", "audio/mp4"],
  ["mp4a", "audio/mp4"],
  ["m2a", "audio/mpeg"],
  ["m3a", "audio/mpeg"],
  ["mp2", "audio/mpeg"],
  ["mp2a", "audio/mpeg"],
  ["mp3", "audio/mpeg"],
  ["mpga", "audio/mpeg"],
  ["oga", "audio/ogg"],
  ["ogg", "audio/ogg"],
  ["opus", "audio/ogg"],
  ["spx", "audio/ogg"],
  ["s3m", "audio/s3m"],
  ["sil", "audio/silk"],
  ["wav", "audio/wav"],
  ["*wav", "audio/wave"],
  ["weba", "audio/webm"],
  ["xm", "audio/xm"],
  ["ttc", "font/collection"],
  ["otf", "font/otf"],
  ["ttf", "font/ttf"],
  ["woff", "font/woff"],
  ["woff2", "font/woff2"],
  ["exr", "image/aces"],
  ["apng", "image/apng"],
  ["avif", "image/avif"],
  ["bmp", "image/bmp"],
  ["cgm", "image/cgm"],
  ["drle", "image/dicom-rle"],
  ["emf", "image/emf"],
  ["fits", "image/fits"],
  ["g3", "image/g3fax"],
  ["gif", "image/gif"],
  ["heic", "image/heic"],
  ["heics", "image/heic-sequence"],
  ["heif", "image/heif"],
  ["heifs", "image/heif-sequence"],
  ["hej2", "image/hej2k"],
  ["hsj2", "image/hsj2"],
  ["ief", "image/ief"],
  ["jls", "image/jls"],
  ["jp2", "image/jp2"],
  ["jpg2", "image/jp2"],
  ["jpe", "image/jpeg"],
  ["jpeg", "image/jpeg"],
  ["jpg", "image/jpeg"],
  ["jph", "image/jph"],
  ["jhc", "image/jphc"],
  ["jpm", "image/jpm"],
  ["jpf", "image/jpx"],
  ["jpx", "image/jpx"],
  ["jxr", "image/jxr"],
  ["jxra", "image/jxra"],
  ["jxrs", "image/jxrs"],
  ["jxs", "image/jxs"],
  ["jxsc", "image/jxsc"],
  ["jxsi", "image/jxsi"],
  ["jxss", "image/jxss"],
  ["ktx", "image/ktx"],
  ["ktx2", "image/ktx2"],
  ["png", "image/png"],
  ["sgi", "image/sgi"],
  ["svg", "image/svg+xml"],
  ["svgz", "image/svg+xml"],
  ["t38", "image/t38"],
  ["tif", "image/tiff"],
  ["tiff", "image/tiff"],
  ["tfx", "image/tiff-fx"],
  ["webp", "image/webp"],
  ["wmf", "image/wmf"],
  ["disposition-notification", "message/disposition-notification"],
  ["u8msg", "message/global"],
  ["u8dsn", "message/global-delivery-status"],
  ["u8mdn", "message/global-disposition-notification"],
  ["u8hdr", "message/global-headers"],
  ["eml", "message/rfc822"],
  ["mime", "message/rfc822"],
  ["3mf", "model/3mf"],
  ["gltf", "model/gltf+json"],
  ["glb", "model/gltf-binary"],
  ["iges", "model/iges"],
  ["igs", "model/iges"],
  ["mesh", "model/mesh"],
  ["msh", "model/mesh"],
  ["silo", "model/mesh"],
  ["mtl", "model/mtl"],
  ["obj", "model/obj"],
  ["stpx", "model/step+xml"],
  ["stpz", "model/step+zip"],
  ["stpxz", "model/step-xml+zip"],
  ["stl", "model/stl"],
  ["vrml", "model/vrml"],
  ["wrl", "model/vrml"],
  ["*x3db", "model/x3d+binary"],
  ["x3dbz", "model/x3d+binary"],
  ["x3db", "model/x3d+fastinfoset"],
  ["*x3dv", "model/x3d+vrml"],
  ["x3dvz", "model/x3d+vrml"],
  ["x3d", "model/x3d+xml"],
  ["x3dz", "model/x3d+xml"],
  ["x3dv", "model/x3d-vrml"],
  ["appcache", "text/cache-manifest"],
  ["manifest", "text/cache-manifest"],
  ["ics", "text/calendar"],
  ["ifb", "text/calendar"],
  ["coffee", "text/coffeescript"],
  ["litcoffee", "text/coffeescript"],
  ["css", "text/css"],
  ["csv", "text/csv"],
  ["htm", "text/html"],
  ["html", "text/html"],
  ["shtml", "text/html"],
  ["jade", "text/jade"],
  ["jsx", "text/jsx"],
  ["less", "text/less"],
  ["markdown", "text/markdown"],
  ["md", "text/markdown"],
  ["mml", "text/mathml"],
  ["mdx", "text/mdx"],
  ["n3", "text/n3"],
  ["conf", "text/plain"],
  ["def", "text/plain"],
  ["in", "text/plain"],
  ["ini", "text/plain"],
  ["list", "text/plain"],
  ["log", "text/plain"],
  ["text", "text/plain"],
  ["txt", "text/plain"],
  ["rtx", "text/richtext"],
  ["*rtf", "text/rtf"],
  ["sgm", "text/sgml"],
  ["sgml", "text/sgml"],
  ["shex", "text/shex"],
  ["slim", "text/slim"],
  ["slm", "text/slim"],
  ["spdx", "text/spdx"],
  ["styl", "text/stylus"],
  ["stylus", "text/stylus"],
  ["tsv", "text/tab-separated-values"],
  ["man", "text/troff"],
  ["me", "text/troff"],
  ["ms", "text/troff"],
  ["roff", "text/troff"],
  ["t", "text/troff"],
  ["tr", "text/troff"],
  ["ttl", "text/turtle"],
  ["uri", "text/uri-list"],
  ["uris", "text/uri-list"],
  ["urls", "text/uri-list"],
  ["vcard", "text/vcard"],
  ["vtt", "text/vtt"],
  ["*xml", "text/xml"],
  ["yaml", "text/yaml"],
  ["yml", "text/yaml"],
  ["3gp", "video/3gpp"],
  ["3gpp", "video/3gpp"],
  ["3g2", "video/3gpp2"],
  ["h261", "video/h261"],
  ["h263", "video/h263"],
  ["h264", "video/h264"],
  ["m4s", "video/iso.segment"],
  ["jpgv", "video/jpeg"],
  ["jpm", "video/jpm"],
  ["jpgm", "video/jpm"],
  ["mj2", "video/mj2"],
  ["mjp2", "video/mj2"],
  ["ts", "application/typescript"],
  ["mp4", "video/mp4"],
  ["mp4v", "video/mp4"],
  ["mpg4", "video/mp4"],
  ["m1v", "video/mpeg"],
  ["m2v", "video/mpeg"],
  ["mpe", "video/mpeg"],
  ["mpeg", "video/mpeg"],
  ["mpg", "video/mpeg"],
  ["ogv", "video/ogg"],
  ["mov", "video/quicktime"],
  ["qt", "video/quicktime"],
  ["webm", "video/webm"]
]);
// crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/urlMatch.ts
var escapedChars = new Set(["$", "^", "+", ".", "*", "(", ")", "|", "\\", "?", "{", "}", "[", "]"]);
// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-v1.ts
var INTERACTIVE = new Set([
  "button",
  "link",
  "textbox",
  "checkbox",
  "radio",
  "combobox",
  "listbox",
  "menuitem",
  "menuitemcheckbox",
  "menuitemradio",
  "option",
  "searchbox",
  "slider",
  "spinbutton",
  "switch",
  "tab",
  "treeitem"
]);
var FIELD = new Set([
  "textbox",
  "searchbox",
  "spinbutton",
  "slider",
  "combobox",
  "listbox",
  "checkbox",
  "radio",
  "switch",
  "menuitemcheckbox",
  "menuitemradio"
]);
var SECTION = new Set([
  "banner",
  "navigation",
  "main",
  "complementary",
  "contentinfo",
  "search",
  "form",
  "region",
  "article",
  "iframe"
]);
var KEPT = new Set([
  "dialog",
  "alertdialog",
  "alert",
  "status"
]);
var KEPT_WITH_REF = new Set(["dialog", "alertdialog"]);
function shownText(node) {
  return node.role === "iframe" ? undefined : node.text;
}
function sectionLabel(node) {
  return node.name ? `${node.role} ${JSON.stringify(node.name)}` : node.role;
}
function keptRef(node) {
  return KEPT_WITH_REF.has(node.role) || actionable(node);
}
var TEXT_LIMIT_CHARS = 3000 * CHARS_PER_TOKEN;
var DEFAULT_MAX_TOKENS = 8000;
var OWN_TEXT_CLIP = 80;
var LONG_SELECT = 12;
var SEPARATORS_ONLY = /^[\s|·•‧∙/\\\-–—‐›»«‹><:;,.()[\]{}]*$/;
function withDefaults(passed) {
  return {
    text: passed.text,
    urls: passed.urls ?? false,
    maxTokens: passed.maxTokens ?? DEFAULT_MAX_TOKENS,
    ...passed.inert ? { inert: true } : {}
  };
}
function renderSnapshotV1(result, options) {
  const { urls, maxTokens } = options;
  const textForm = options.text === false ? null : walk(result, urls, true, options.inert === true);
  let text;
  if (textForm === null)
    text = "never";
  else if (options.text === true)
    text = "always";
  else
    text = joinedLength(textForm.lines) <= TEXT_LIMIT_CHARS ? "kept" : "leftOut";
  const form = {
    text,
    urls,
    maxTokens,
    ...options.inert ? { inert: true } : {}
  };
  return { text: renderForm(result, form, textForm), form };
}
function renderSnapshotV1As(result, form) {
  return renderForm(result, form, null);
}
function renderForm(result, form, walked) {
  const limit = form.maxTokens * CHARS_PER_TOKEN;
  const text = form.text === "kept" || form.text === "always";
  const shown = text && walked ? walked : walk(result, form.urls, text, form.inert === true);
  const omitted = shown.omitted;
  const inertHead = shown.skipped ? [
    `# not listed: ${omitted.length} lines (${omitted.filter((line) => line.ref).length} with refs) of this page are inert: nothing in them can be clicked or typed into now. Pass { inert: true } to list them.`
  ] : [];
  const partial = partialHead(result);
  const finish = (head, lines) => {
    const body = cap(head, partial, lines, limit);
    return [
      ...onTopHead(result),
      ...unmarkedHead(result),
      ...inertHead,
      ...body ? [body] : []
    ].join(`
`);
  };
  if (form.text !== "leftOut")
    return finish([], shown.lines);
  const textForm = walked ?? walk(result, form.urls, true, form.inert === true);
  const left = joinedLength(textForm.lines) - joinedLength(shown.lines);
  const about = aboutTokens(left);
  const notice = `# text left out: about ${about} tokens. Pass { text: true } to include it.`;
  return finish([notice], shown.lines);
}
function walk(result, urls, text, inert) {
  const partials = new Map;
  for (const partial of result.partial)
    partials.set(partial.ref, [...partials.get(partial.ref) ?? [], partial]);
  const omitted = [];
  let segment;
  let boundary;
  let skipped = 0;
  const printer = new Printer(urls, partials, inert, (node, text, level) => {
    skipped++;
    const line = printer.lines.at(-1);
    if (!segment || !boundary || boundary.line !== line || boundary.body !== line?.body || boundary.length !== printer.lines.length || boundary.level !== level) {
      segment = new Printer(urls, new Map, true);
      omitted.push(segment);
    }
    segment.walk([node], level, text);
    boundary = { line, body: line?.body, length: printer.lines.length, level };
  });
  const front = onTopContent(result);
  if (front.first.length) {
    printer.walk(front.first, 0, true);
    printer.separateText();
  }
  printer.walk(front.rest, 0, text);
  for (const unplaced of partials.values())
    for (const partial of unplaced)
      printer.comment(partialComment(partial), 0);
  return {
    lines: printedLines(printer.finish()),
    omitted: omitted.flatMap((segment) => printedLines(segment.finish())),
    skipped
  };
}
function printedLines(lines) {
  return lines.map((line) => ({
    text: "  ".repeat(line.level) + (line.textLine ? renderAriaSnapshotAsYaml([{ role: "text", text: line.body }]) : line.body),
    ref: line.ref,
    section: line.section,
    keptEnd: line.keptEnd
  }));
}

class Printer {
  urls;
  partials;
  inert;
  onOmit;
  lines = [];
  steps = [];
  textBoundary = 0;
  constructor(urls, partials, inert, onOmit) {
    this.urls = urls;
    this.partials = partials;
    this.inert = inert;
    this.onOmit = onOmit;
  }
  walk(children, level, text) {
    this.content({ children: [...children] }, level, text);
    for (let step = this.steps.pop();step; step = this.steps.pop()) {
      if (step.kind === "node")
        this.node(step.child, step.level, step.text);
      else if (step.kind === "piece")
        this.piece(step.raw, step.level, step.text);
      else if (step.kind === "kept")
        this.closeKept(step.node, step.header, step.level);
      else
        this.closeSection(step.node, step.header);
    }
  }
  separateText() {
    this.textBoundary = this.lines.length;
  }
  comment(body, level) {
    this.lines.push({ level, body, textLine: false, comment: true, ref: false });
  }
  finish() {
    const printed = [];
    const moved = [];
    for (const line of this.lines) {
      moved.push(printed.length);
      if (!separatorsOnly(line))
        printed.push(line);
    }
    moved.push(printed.length);
    for (const line of printed) {
      if (line.section)
        line.section.end = moved[line.section.end];
      if (line.keptEnd !== undefined)
        line.keptEnd = moved[line.keptEnd];
    }
    return printed;
  }
  omit(root) {
    const pending = [root];
    while (pending.length) {
      const node = pending.pop();
      if (!node)
        continue;
      if (node.ref)
        this.partials.delete(node.ref);
      for (const child of node.children ?? [])
        if (typeof child !== "string")
          pending.push(child);
    }
  }
  node(child, level, text) {
    if (typeof child !== "string" && child.inert && !this.inert) {
      this.onOmit?.(child, text, level);
      this.omit(child);
      return;
    }
    if (typeof child === "string")
      this.piece(child, level, text);
    else if (child.role === "text")
      this.piece(child.text ?? "", level, text);
    else if (KEPT.has(child.role))
      this.kept(child, level);
    else if (SECTION.has(child.role))
      this.section(child, level, text);
    else if (child.role === "heading")
      this.heading(child, level);
    else if (actionable(child) || fieldWithoutRef(child))
      this.control(child, level, text);
    else {
      if (child.role === "img") {
        if (child.name && text)
          this.element(copy(child, { ref: false, urls: false }), level);
      } else {
        const named = nameAsContent(child);
        if (named !== undefined)
          this.piece(named, level, text);
      }
      this.content(child, level, text);
    }
  }
  content(node, level, text) {
    const children = node.children ?? [];
    for (let index = children.length - 1;index >= 0; index--)
      this.steps.push({ kind: "node", child: children[index], level, text });
    const own = "role" in node ? shownText(node) : undefined;
    if (own !== undefined)
      this.steps.push({ kind: "piece", raw: own, level, text });
  }
  piece(raw, level, text) {
    if (!text || raw === "")
      return;
    const last = this.lines[this.lines.length - 1];
    if (this.lines.length > this.textBoundary && last?.textLine && last.level === level)
      last.body += ` ${raw}`;
    else
      this.lines.push({
        level,
        body: raw,
        textLine: true,
        comment: false,
        ref: false
      });
  }
  kept(node, level) {
    const header = this.header(node, level, keptRef(node));
    this.steps.push({ kind: "kept", node, header, level });
    this.content(node, level + 1, true);
  }
  closeKept(node, header, level) {
    const inside = this.lines.slice(header + 1);
    if (inside.length === 1 && inside[0].textLine && !separatorsOnly(inside[0])) {
      this.lines.length = header;
      this.element(copy(node, { ref: keptRef(node), urls: false, text: inside[0].body }), level);
    } else if (!this.close(header, Boolean(node.name)))
      return;
    this.lines[header].keptEnd = this.lines.length;
  }
  section(node, level, text) {
    const header = this.header(node, level, true);
    if (node.role === "iframe" && node.ref !== undefined) {
      for (const partial of this.partials.get(node.ref) ?? [])
        this.comment(partialComment(partial), level + 1);
      this.partials.delete(node.ref);
    }
    this.steps.push({ kind: "section", node, header });
    this.content(node, level + 1, text);
  }
  closeSection(node, header) {
    if (this.close(header, node.role === "iframe") && this.lines[header].body.endsWith(":") && node.ref !== undefined)
      this.lines[header].section = {
        ref: node.ref,
        label: sectionLabel(node),
        end: this.lines.length
      };
  }
  heading(node, level) {
    const own = node.name ? "" : ownText(node, this.inert);
    this.element(copy(node, {
      ref: actionable(node),
      urls: false,
      text: own === "" ? undefined : own
    }), level);
    this.content(node, level, false);
  }
  control(node, level, text) {
    let value;
    if (node.text !== undefined && node.text !== node.name)
      value = node.text;
    else if (FIELD.has(node.role) && node.text === undefined) {
      const own = ownText(node, this.inert);
      if (own !== "" && own !== node.name)
        value = own;
    } else if (!node.name) {
      const label = ownText(node, this.inert) || (node.role === "link" && !this.urls ? node.url ?? "" : "");
      if (label !== "")
        value = text ? label : clip(label, OWN_TEXT_CLIP);
    }
    const line = copy(node, { ref: true, urls: this.urls, text: value });
    const choices = node.role === "combobox" || node.role === "listbox" ? nativeOptions(node) : [];
    if (choices.length === 0)
      this.element(line, level);
    else {
      this.header(node, level, true, line);
      this.lines[this.lines.length - 1].body += ":";
      const shown = text || choices.length <= LONG_SELECT ? choices : choices.filter((option) => option.selected === true);
      for (const option of shown)
        this.element(copy(option, { ref: false, urls: false, text: option.text }), level + 1);
      if (shown.length < choices.length)
        this.lines.push({
          level: level + 1,
          body: `- text: ${choices.length - shown.length} more options`,
          textLine: false,
          comment: false,
          ref: false
        });
    }
    this.content(node, level, false);
  }
  element(line, level) {
    const bodies = renderAriaSnapshotAsYaml([line]).split(`
`);
    for (const [index, body] of bodies.entries())
      this.lines.push({
        level,
        body,
        textLine: false,
        comment: false,
        ref: index === 0 && line.ref !== undefined
      });
  }
  header(node, level, ref, line = copy(node, { ref, urls: false })) {
    const { text: _text, url: _url, placeholder: _placeholder, ...key } = line;
    this.lines.push({
      level,
      body: renderAriaSnapshotAsYaml([key]),
      textLine: false,
      comment: false,
      ref: key.ref !== undefined
    });
    return this.lines.length - 1;
  }
  close(header, keepEmpty) {
    const inside = this.lines.slice(header + 1);
    if (inside.some((line) => !line.comment && !separatorsOnly(line))) {
      this.lines[header].body += ":";
      return true;
    }
    if (keepEmpty || inside.some((line) => line.comment))
      return true;
    this.lines.length = header;
    return false;
  }
}
function actionable(node) {
  return node.ref !== undefined && (INTERACTIVE.has(node.role) || node.cursor === "pointer");
}
function fieldWithoutRef(node) {
  return node.ref === undefined && FIELD.has(node.role);
}
function copy(node, keep) {
  const { children: _children, ref, cursor, url, text: _text, ...line } = node;
  const copied = { ...line };
  if (keep.ref && ref !== undefined) {
    copied.ref = ref;
    if (cursor !== undefined && !INTERACTIVE.has(node.role))
      copied.cursor = cursor;
  }
  if (keep.urls && url !== undefined)
    copied.url = url;
  if (keep.text !== undefined)
    copied.text = keep.text;
  return copied;
}
function nativeOptions(select) {
  const options = [];
  const visit = (children) => {
    for (const child of children ?? []) {
      if (typeof child === "string")
        continue;
      if (child.role === "option" && !child.ref)
        options.push(child);
      else if (child.role === "group" && !actionable(child))
        visit(child.children);
    }
  };
  visit(select.children);
  return options;
}
function separatorsOnly(line) {
  return line.textLine && SEPARATORS_ONLY.test(line.body);
}
function nameAsContent(node) {
  if (!node.name || node.text !== undefined || node.role === "heading" || fieldWithoutRef(node))
    return;
  return saysNothing(node.children) ? node.name : undefined;
}
function saysNothing(children) {
  const pending = [];
  const add = (nodes) => {
    for (const node of nodes ?? [])
      pending.push(node);
  };
  add(children);
  for (let item = pending.pop();item !== undefined; item = pending.pop()) {
    if (typeof item === "string") {
      if (item !== "")
        return false;
    } else if (shownText(item) || item.name)
      return false;
    else
      add(item.children);
  }
  return true;
}
function ownText(node, inert) {
  const pieces = [];
  const stack = [node];
  for (let item = stack.pop();item !== undefined; item = stack.pop()) {
    if (typeof item === "string") {
      pieces.push(item);
      continue;
    }
    const shown = shownText(item);
    if (shown !== undefined)
      pieces.push(shown);
    const children = item.children ?? [];
    for (let index = children.length - 1;index >= 0; index--) {
      const child = children[index];
      if (typeof child !== "string" && child.inert && !inert)
        continue;
      if (typeof child === "string" || child.role === "text")
        stack.push(child);
      else if (!actionable(child) && !fieldWithoutRef(child))
        stack.push(nameAsContent(child) ?? child);
    }
  }
  return pieces.filter((piece) => piece !== "").join(" ");
}
function clip(text, length) {
  if (text.length <= length)
    return text;
  const last = text.charCodeAt(length - 1);
  if (last >= 55296 && last <= 56319)
    length--;
  return `${text.slice(0, length).trimEnd()}…`;
}
function partialComment(partial) {
  return `# partial: ${partial.frame} not read (${partial.reason})`;
}
function joinedLength(lines) {
  return lines.reduce((length, line, index) => length + line.text.length + (index > 0 ? 1 : 0), 0);
}

// crates/neo-sdk/neo-playwright-js/src/prelude/output-policy.ts
var upstreamSnapshots = Reflect.get(globalThis, "__neo_upstream_snapshots") === true;

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot.ts
var FULL_CALL = "ariaSnapshot({ mode: 'full' })";
function comment(partial) {
  return `# partial: ${partial.frame} not read (${partial.reason})`;
}
function treeLines(json, keepFallback) {
  const lines = [];
  const steps = [];
  const later = (children, depth) => {
    for (let index = children.length - 1;index >= 0; index--)
      steps.push({ child: children[index], depth });
  };
  later(json, 0);
  for (let step = steps.pop();step; step = steps.pop()) {
    const indent = "  ".repeat(step.depth);
    const { child } = step;
    if (typeof child === "string" || child.role === "text") {
      const text = typeof child === "string" ? child : child.text || "";
      const line = renderAriaSnapshotAsYaml([{ role: "text", text }]);
      if (line !== "")
        lines.push({ text: indent + line });
      continue;
    }
    const { children, text: _text, ...own } = child;
    const text = keepFallback ? child.text : shownText(child);
    const alone = text === undefined ? own : { ...own, text };
    const [key, ...rest] = renderAriaSnapshotAsYaml([alone]).split(`
`);
    const block = text === undefined && own.url === undefined && own.placeholder === undefined && (children?.length ?? 0) > 0;
    lines.push({ text: indent + key + (block ? ":" : ""), node: child });
    for (const line of rest)
      lines.push({ text: indent + line });
    if (text === undefined && children)
      later(children, step.depth + 1);
  }
  return lines;
}
function iframeLine(lines, ref) {
  const placed = lines.findIndex((line) => line.node?.ref === ref);
  if (placed !== -1)
    return placed;
  const tag = `[ref=${ref}]`;
  return lines.findIndex((line) => /^\s*- /.test(line.text) && line.text.includes(tag));
}
function yamlLines(result, keepFallback) {
  const lines = treeLines(result.json, keepFallback);
  if (result.partial.length === 0)
    return lines;
  const placed = result.partial.map((partial) => ({
    partial,
    line: iframeLine(lines, partial.ref)
  }));
  const unplaced = [];
  for (const { partial, line } of placed.sort((a, b) => b.line - a.line)) {
    if (line === -1) {
      unplaced.unshift({ text: comment(partial) });
      continue;
    }
    const indent = /^\s*/.exec(lines[line]?.text ?? "")?.[0] ?? "";
    lines.splice(line + 1, 0, { text: `${indent}  ${comment(partial)}` });
  }
  return [...lines, ...unplaced];
}
function renderSnapshot(result) {
  return yamlLines(result, true).map((line) => line.text).join(`
`);
}
function renderFullSnapshot(result, maxTokens) {
  const roots = inertRoots(result.json);
  const count = roots.length ? treeLines(roots, false).length : 0;
  const head = count ? [
    `# inert: ${count} lines of this page cannot be clicked or typed into now. page.ariaSnapshot() lists what can.`
  ] : [];
  const body = cap([], partialHead(result), printed(yamlLines(result, false)), maxTokens * CHARS_PER_TOKEN, FULL_CALL);
  return [
    ...onTopHead(result),
    ...unmarkedHead(result),
    ...head,
    ...body ? [body] : []
  ].join(`
`);
}
function printed(lines) {
  const ends = lines.map(() => lines.length);
  const open = [];
  for (let index = 0;index < lines.length; index++) {
    const text = lines[index].text;
    const indent = text.length - text.trimStart().length;
    for (let last = open.at(-1);last !== undefined && last.indent >= indent; last = open.at(-1)) {
      ends[last.index] = index;
      open.pop();
    }
    open.push({ index, indent });
  }
  return lines.map(({ text, node }, index) => {
    const line = { text, ref: text.includes("[ref=") };
    const end = ends[index];
    if (node === undefined)
      return line;
    if (KEPT.has(node.role))
      line.keptEnd = end;
    else if (SECTION.has(node.role) && node.ref !== undefined && end > index + 1)
      line.section = { ref: node.ref, label: sectionLabel(node), end };
    return line;
  });
}

// crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-delta.ts
var NO_BASELINE = "# whole page: no earlier snapshot of this page";
var NO_CHANGE = "# no change since your last snapshot of this tab";
var TOO_MUCH = "# whole page: too much differs from your last snapshot to list as changes";
var NO_TIME = "# whole page: no time was left to look for an earlier snapshot of this page";
var CONTEXT = 3;
var MAX_CHANGED_LINES = 1000;
function renderDelta(stored, result, passed) {
  if (stored === null) {
    const whole = renderSnapshotV1(result, withDefaults(passed));
    return {
      returned: [
        NO_BASELINE,
        ...onTopHead(result),
        ...unmarkedHead(result),
        withoutReadHeads(whole.text)
      ].join(`
`),
      baseline: whole
    };
  }
  const { form } = stored;
  if (agrees(passed, form)) {
    const current = renderSnapshotV1As(result, form);
    const changes = listChanges(stored.text, current);
    if (changes !== null) {
      const [status, ...body] = changes.split(`
`);
      return {
        returned: [
          status,
          ...onTopDelta(stored.text, result),
          ...unmarkedHead(result),
          ...body
        ].join(`
`),
        baseline: { text: current, form }
      };
    }
  }
  const whole = renderSnapshotV1(result, {
    text: passed.text ?? passedText(form.text),
    urls: passed.urls ?? form.urls,
    maxTokens: passed.maxTokens ?? form.maxTokens,
    inert: passed.inert ?? form.inert ?? false
  });
  return {
    returned: [
      TOO_MUCH,
      ...onTopDelta(stored.text, result),
      ...unmarkedHead(result),
      withoutReadHeads(whole.text)
    ].join(`
`),
    baseline: whole
  };
}
function renderUncompared(result, passed) {
  return [
    NO_TIME,
    ...onTopHead(result),
    ...unmarkedHead(result),
    withoutReadHeads(renderSnapshotV1(result, withDefaults(passed)).text)
  ].join(`
`);
}
function agrees(passed, form) {
  return (passed.text === undefined || passed.text === showsText(form.text)) && (passed.urls === undefined || passed.urls === form.urls) && (passed.maxTokens === undefined || passed.maxTokens === form.maxTokens) && (passed.inert === undefined || passed.inert === (form.inert ?? false));
}
function showsText(text) {
  return text === "kept" || text === "always";
}
function passedText(text) {
  if (text === "always")
    return true;
  if (text === "never")
    return false;
  return;
}
function sizeLine(line) {
  return isOnTopLine(line) || isUnmarkedLine(line) || line.startsWith("# text left out: ") || line.startsWith("# cut: ") || line.startsWith("# not listed: ");
}
function listChanges(previous, current) {
  const before = previous.split(`
`).filter((line) => !sizeLine(line));
  const after = current.split(`
`).filter((line) => !sizeLine(line));
  const edits = diff(before, after, current.length);
  if (edits === null)
    return null;
  if (edits.every((edit) => edit.kind === " "))
    return NO_CHANGE;
  const changes = hunks(edits, before, after).join(`
`);
  if (changes.length > current.length / 2)
    return null;
  const added = edits.filter((edit) => edit.kind === "+").length;
  const removed = edits.filter((edit) => edit.kind === "-").length;
  return `# changes since your last snapshot of this tab: ${added} added, ${removed} removed
${changes}`;
}
function diff(before, after, limit) {
  let head = 0;
  while (head < before.length && head < after.length && before[head] === after[head])
    head += 1;
  let tail = 0;
  while (tail < before.length - head && tail < after.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail])
    tail += 1;
  const n = before.length - head - tail;
  const m = after.length - head - tail;
  if (unmatchedCost(before, after, head, n, m) > limit / 2)
    return null;
  let shortest = Number.POSITIVE_INFINITY;
  for (let index = head;index < head + n; index++)
    shortest = Math.min(shortest, before[index].length);
  for (let index = head;index < head + m; index++)
    shortest = Math.min(shortest, after[index].length);
  const most = Math.min(MAX_CHANGED_LINES, n + m, n + m === 0 ? 0 : Math.floor(limit / 2 / (shortest + 2)));
  const middle = middleScript(before, after, head, n, m, most);
  if (middle === null)
    return null;
  const edits = [];
  for (let index = 0;index < head; index++)
    edits.push({ kind: " ", before: index, after: index });
  edits.push(...middle);
  for (let index = 0;index < tail; index++)
    edits.push({
      kind: " ",
      before: head + n + index,
      after: head + m + index
    });
  return edits;
}
function unmatchedCost(before, after, offset, n, m) {
  const spare = new Map;
  for (let index = offset;index < offset + m; index++)
    spare.set(after[index], (spare.get(after[index]) ?? 0) + 1);
  let cost = 0;
  for (let index = offset;index < offset + n; index++) {
    const left = spare.get(before[index]) ?? 0;
    if (left > 0)
      spare.set(before[index], left - 1);
    else
      cost += before[index].length + 2;
  }
  for (const [line, left] of spare)
    cost += left * (line.length + 2);
  return cost;
}
function middleScript(before, after, offset, n, m, most) {
  const width = 2 * most + 3;
  const reach = new Int32Array(width);
  const rounds = [];
  const at = (k) => k + most + 1;
  for (let d = 0;d <= most; d++) {
    for (let k = -d;k <= d; k += 2) {
      const down = k === -d || k !== d && reach[at(k - 1)] < reach[at(k + 1)];
      let x = down ? reach[at(k + 1)] : reach[at(k - 1)] + 1;
      let y = x - k;
      while (x < n && y < m && before[offset + x] === after[offset + y]) {
        x += 1;
        y += 1;
      }
      reach[at(k)] = x;
      if (x >= n && y >= m) {
        rounds.push(reach.slice(at(-d), at(d) + 1));
        return walkBack(rounds, n, m, offset);
      }
    }
    rounds.push(reach.slice(at(-d), at(d) + 1));
  }
  return null;
}
function walkBack(rounds, n, m, offset) {
  const reversed = [];
  let x = n;
  let y = m;
  for (let d = rounds.length - 1;d > 0; d--) {
    const k = x - y;
    const previous = rounds[d - 1];
    const at = (diagonal) => diagonal + d - 1;
    const down = k === -d || k !== d && previous[at(k - 1)] < previous[at(k + 1)];
    const fromK = down ? k + 1 : k - 1;
    const fromX = previous[at(fromK)];
    const fromY = fromX - fromK;
    while (x > (down ? fromX : fromX + 1) && y > (down ? fromY + 1 : fromY)) {
      x -= 1;
      y -= 1;
      reversed.push({ kind: " ", before: offset + x, after: offset + y });
    }
    if (down) {
      y -= 1;
      reversed.push({ kind: "+", before: offset + x, after: offset + y });
    } else {
      x -= 1;
      reversed.push({ kind: "-", before: offset + x, after: offset + y });
    }
  }
  while (x > 0 && y > 0) {
    x -= 1;
    y -= 1;
    reversed.push({ kind: " ", before: offset + x, after: offset + y });
  }
  return reversed.reverse();
}
function hunks(edits, before, after) {
  const out = [];
  let index = 0;
  while (index < edits.length) {
    if (edits[index].kind === " ") {
      index += 1;
      continue;
    }
    const start = Math.max(0, index - CONTEXT);
    let end = index;
    let quiet = 0;
    for (let next = index;next < edits.length; next++) {
      if (edits[next].kind === " ") {
        quiet += 1;
        if (quiet > 2 * CONTEXT)
          break;
      } else {
        quiet = 0;
        end = next;
      }
    }
    const stop = Math.min(edits.length, end + 1 + CONTEXT);
    const lines = edits.slice(start, stop);
    out.push(blockLine(lines, before, after));
    for (const edit of lines)
      out.push(edit.kind + (edit.kind === "+" ? after[edit.after] : before[edit.before]));
    index = stop;
  }
  return out;
}
function blockLine(lines, before, after) {
  const first = lines.find((edit) => edit.kind !== " ");
  if (first === undefined)
    return "@@";
  const removesOnly = lines.every((edit) => edit.kind !== "+");
  const text = removesOnly ? before : after;
  const position = removesOnly ? first.before : first.after;
  const changed = first.kind === "+" ? after[first.after] : before[first.before];
  const depth = indentOf(changed);
  for (let index = position - 1;index >= 0; index--) {
    if (indentOf(text[index]) < depth)
      return `@@ ${text[index].trimStart().replace(/^- /, "").replace(/:$/, "")}`;
  }
  return "@@";
}
function indentOf(line) {
  return line.length - line.trimStart().length;
}
function withoutReadHeads(text) {
  return text.split(`
`).filter((line) => !isOnTopLine(line) && !isUnmarkedLine(line)).join(`
`);
}

// crates/neo-sdk/neo-playwright-js/scripts/snapshot-cases/snapshot-lines.ts
var SNAPSHOT_CHARS = 1500;
function snapshotLines(snapshot) {
  if (typeof snapshot !== "object" || snapshot === null)
    return "";
  let text;
  try {
    text = renderSnapshotV1({ json: snapshot, partial: [] }, withDefaults({})).text;
  } catch {
    return "";
  }
  const kept = [];
  let used = 0;
  for (const line of text.split(`
`)) {
    if (line.trim() === "")
      continue;
    if (used + line.length + 1 > SNAPSHOT_CHARS) {
      const room = SNAPSHOT_CHARS - used - 1;
      if (room > 0)
        kept.push(`    ${line.slice(0, room)}…`);
      kept.push("    … (the rest of its snapshot is cut)");
      break;
    }
    used += line.length + 1;
    kept.push(`    ${line}`);
  }
  return kept.join(`
`);
}

// crates/neo-sdk/neo-playwright-js/scripts/snapshot-cases/answer.ts
var COVER_HEAD = `locator.click: the element is covered
  On top: dialog "Cookies" [ref=e9]`;
var COVER_TAIL = `
  Its controls: button "Accept" [ref=e10]`;
function coverBlock(json) {
  const lines = snapshotLines(json);
  const error = new Error(COVER_HEAD + COVER_TAIL);
  Reflect.set(error, "cover", {
    ref: "e9",
    role: "dialog",
    name: "Cookies",
    kind: "dialog",
    controls: [],
    ...lines === "" ? {} : { block: lines }
  });
  shapeCovered(error);
  if (error.message === COVER_HEAD + COVER_TAIL)
    return null;
  return error.message.slice(COVER_HEAD.length + 1, -COVER_TAIL.length);
}
function todaysAnswer(call, result) {
  if ("cover" in call)
    return { text: coverBlock(result?.json ?? null) };
  if (result === null)
    throw new Error("only a cover may have no snapshot");
  if ("playwright" in call)
    return { text: renderSnapshot(result) };
  if ("full" in call)
    return {
      text: renderFullSnapshot(result, call.full.maxTokens ?? DEFAULT_MAX_TOKENS)
    };
  if ("element" in call)
    return { text: renderSnapshotV1(result, withDefaults(call.element)).text };
  if ("page" in call) {
    const { text, form } = renderSnapshotV1(result, withDefaults(call.page));
    return { text, pending: { text, form } };
  }
  const { passed, earlier: found } = call.delta;
  if (found === "notRead")
    return { text: renderUncompared(result, passed) };
  const answer = renderDelta(found === "nothing" ? null : found, result, passed);
  return {
    text: answer.returned,
    pending: { text: answer.baseline.text, form: answer.baseline.form }
  };
}
globalThis.todaysAnswer = todaysAnswer;
})();
