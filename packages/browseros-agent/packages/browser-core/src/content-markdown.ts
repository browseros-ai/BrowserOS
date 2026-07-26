export interface ContentMarkdownOptions {
  selector?: string
  viewportOnly?: boolean
  includeLinks?: boolean
  includeImages?: boolean
}

export function buildContentMarkdownExpression(
  opts: ContentMarkdownOptions,
): string {
  return `${DOM_WALKER_SCRIPT}(${JSON.stringify({
    selector: opts.selector,
    viewport: opts.viewportOnly ?? false,
    links: opts.includeLinks ?? true,
    images: opts.includeImages ?? false,
  })})`
}

// Injected into page via Runtime.evaluate.
// Uses var + ES5 style for consistency with other injected scripts.
// Context object: { pre: bool, ld: listDepth, lt: listType, td: tableDepth }
const DOM_WALKER_SCRIPT = `(function(o) {
var SKIP = {SCRIPT:1,STYLE:1,NOSCRIPT:1,SVG:1,TEMPLATE:1,CANVAS:1,VIDEO:1,AUDIO:1,OBJECT:1,EMBED:1};
var FORM = {INPUT:1,SELECT:1,TEXTAREA:1,BUTTON:1};
var vh = window.innerHeight, vw = window.innerWidth;
var root = o.selector ? document.querySelector(o.selector) : document.body;
if (!root) return '';

function hidden(el) {
  if (el.hidden || el.getAttribute('aria-hidden') === 'true') return true;
  var s = getComputedStyle(el);
  return s.display === 'none' || s.visibility === 'hidden';
}

function outOfView(el) {
  if (!o.viewport) return false;
  var r = el.getBoundingClientRect();
  return r.bottom <= 0 || r.top >= vh || r.right <= 0 || r.left >= vw;
}

function kids(el, ctx) {
  var r = '';
  for (var i = 0; i < el.childNodes.length; i++)
    r += walk(el.childNodes[i], ctx);
  return r;
}

function fmtTable(tableEl, ctx) {
  var rows = [];
  var sepAfter = 0;
  var cellCtx = {pre: false, ld: 0, lt: 'ul', td: ctx.td + 1};

  function addTr(tr, inHead) {
    var cells = [];
    var isHeader = inHead;
    for (var i = 0; i < tr.children.length; i++) {
      var c = tr.children[i];
      var tag = c.tagName;
      if (tag === 'TH' || tag === 'TD') {
        var txt = walk(c, cellCtx).trim()
          .replace(/\\n+/g, ' ').replace(/ {2,}/g, ' ').replace(/\\|/g, '\\\\|');
        cells.push(txt);
        if (tag === 'TH') isHeader = true;
      }
    }
    if (cells.length) {
      if (isHeader) sepAfter = rows.length;
      rows.push(cells);
    }
  }

  for (var i = 0; i < tableEl.children.length; i++) {
    var sec = tableEl.children[i];
    var tag = sec.tagName;
    if (tag === 'THEAD' || tag === 'TBODY' || tag === 'TFOOT') {
      for (var j = 0; j < sec.children.length; j++)
        if (sec.children[j].tagName === 'TR') addTr(sec.children[j], tag === 'THEAD');
    } else if (tag === 'TR') {
      addTr(sec, false);
    }
  }

  if (!rows.length) return '';

  var mc = 0;
  for (var i = 0; i < rows.length; i++)
    if (rows[i].length > mc) mc = rows[i].length;
  if (!mc) return '';
  for (var i = 0; i < rows.length; i++)
    while (rows[i].length < mc) rows[i].push('');

  var lines = [];
  for (var i = 0; i <= sepAfter; i++)
    lines.push('| ' + rows[i].join(' | ') + ' |');
  var sep = [];
  for (var j = 0; j < mc; j++) sep.push('---');
  lines.push('| ' + sep.join(' | ') + ' |');
  for (var i = sepAfter + 1; i < rows.length; i++)
    lines.push('| ' + rows[i].join(' | ') + ' |');

  return '\\n\\n' + lines.join('\\n') + '\\n\\n';
}

function walk(node, ctx) {
  if (node.nodeType === 3) {
    if (ctx.pre) return node.textContent || '';
    return (node.textContent || '').replace(/[\\t\\n\\r]+/g, ' ');
  }
  if (node.nodeType !== 1) return '';

  var el = node;
  var T = el.tagName;
  if (SKIP[T] || FORM[T]) return '';
  if (hidden(el)) return '';

  var t;
  switch (T) {
    case 'H1': case 'H2': case 'H3': case 'H4': case 'H5': case 'H6':
      if (o.viewport && outOfView(el)) return '';
      t = kids(el, ctx).trim();
      if (!t) return '';
      var lvl = T.charCodeAt(1) - 48;
      return '\\n\\n' + '#'.repeat(lvl) + ' ' + t + '\\n\\n';

    case 'P':
      if (o.viewport && outOfView(el)) return '';
      t = kids(el, ctx).trim();
      return t ? '\\n\\n' + t + '\\n\\n' : '';

    case 'A':
      t = kids(el, ctx).trim().replace(/\\n+/g, ' ');
      if (!t) {
        var img = el.querySelector('img');
        if (img) t = img.alt || '';
      }
      if (!t) return '';
      if (!o.links) return t;
      var href = el.href;
      if (!href || href.startsWith('javascript:')) return t;
      return '[' + t + '](' + href + ')';

    case 'IMG':
      if (!o.images) return '';
      if (o.viewport && outOfView(el)) return '';
      var src = el.src;
      return src ? '![' + (el.alt || '') + '](' + src + ')' : '';

    case 'STRONG': case 'B':
      t = kids(el, ctx).trim();
      return t ? '**' + t + '**' : '';

    case 'EM': case 'I':
      t = kids(el, ctx).trim();
      return t ? '*' + t + '*' : '';

    case 'DEL': case 'S':
      t = kids(el, ctx).trim();
      return t ? '~~' + t + '~~' : '';

    case 'CODE':
      if (ctx.pre) return kids(el, ctx);
      t = (el.textContent || '').trim();
      return t ? '\\u0060' + t + '\\u0060' : '';

    case 'PRE':
      if (o.viewport && outOfView(el)) return '';
      var codeEl = el.querySelector('code');
      var lang = '';
      if (codeEl) {
        var m = (codeEl.className || '').match(/(?:language|lang)-(\\w+)/);
        if (m) lang = m[1];
      }
      t = el.textContent || '';
      if (t.endsWith('\\n')) t = t.slice(0, -1);
      return '\\n\\n\\u0060\\u0060\\u0060' + lang + '\\n' + t + '\\n\\u0060\\u0060\\u0060\\n\\n';

    case 'BLOCKQUOTE':
      if (o.viewport && outOfView(el)) return '';
      t = kids(el, ctx).trim();
      if (!t) return '';
      return '\\n\\n' + t.split('\\n').map(function(l) { return '> ' + l; }).join('\\n') + '\\n\\n';

    case 'UL': case 'OL':
      var inner = kids(el, {pre: false, ld: ctx.ld + 1, lt: T === 'OL' ? 'ol' : 'ul', td: ctx.td});
      return ctx.ld === 0
        ? '\\n\\n' + inner.trimEnd() + '\\n\\n'
        : '\\n' + inner.trimEnd() + '\\n';

    case 'LI':
      if (o.viewport && outOfView(el)) return '';
      var indent = '  '.repeat(Math.max(0, ctx.ld - 1));
      var bullet = ctx.lt === 'ol' ? '1. ' : '- ';
      t = kids(el, ctx).replace(/^\\s+/, '').trimEnd();
      return t ? indent + bullet + t + '\\n' : '';

    case 'DL':
      return '\\n\\n' + kids(el, ctx).trimEnd() + '\\n\\n';
    case 'DT':
      t = kids(el, ctx).trim();
      return t ? '\\n**' + t + '**\\n' : '';
    case 'DD':
      t = kids(el, ctx).trim();
      return t ? ': ' + t + '\\n' : '';

    case 'TABLE':
      if (o.viewport && outOfView(el)) return '';
      if (ctx.td > 0) return kids(el, ctx);
      return fmtTable(el, ctx);

    case 'BR': return '\\n';
    case 'HR':
      if (o.viewport && outOfView(el)) return '';
      return '\\n\\n---\\n\\n';

    case 'SUMMARY':
      t = kids(el, ctx).trim();
      return t ? '\\n**' + t + '**\\n' : '';
    case 'FIGCAPTION':
      t = kids(el, ctx).trim();
      return t ? '\\n*' + t + '*\\n' : '';

    case 'IFRAME':
      try {
        var idoc = el.contentDocument;
        if (idoc && idoc.body) return walk(idoc.body, ctx);
      } catch(e) {}
      var isrc = el.src || el.getAttribute('src');
      if (isrc) return '\\n\\n[iframe: ' + isrc + ']\\n\\n';
      return '';

    default:
      return kids(el, ctx);
  }
}

var md = walk(root, {pre: false, ld: 0, lt: 'ul', td: 0});
return md.replace(/\\n{3,}/g, '\\n\\n').trim();
})`
