// biome-ignore-all lint: Injected ES5 asset mirrors the TypeScript browser-core runtime string.
// biome-ignore format: Keep injected script byte-oriented and close to the TypeScript source.
(() => {
  var items = __BROWSEROS_ITEMS__;
  var attr = __BROWSEROS_ATTR__;
  var token = __BROWSEROS_TOKEN__;
  var useDocumentSpaceLabels = __BROWSEROS_FULL_PAGE__;
  var scroll = __BROWSEROS_SCROLL__;
  var sx = scroll && typeof scroll.x === 'number' ? scroll.x : window.scrollX || 0;
  var sy = scroll && typeof scroll.y === 'number' ? scroll.y : window.scrollY || 0;
  var c = document.createElement('div');
  c.setAttribute(attr, token);
  c.style.cssText = 'position:absolute;top:0;left:0;width:0;height:0;pointer-events:none;z-index:2147483647;';
  for (var i = 0; i < items.length; i++) {
    var it = items[i];
    var dx = it.x + sx;
    var dy = it.y + sy;
    var b = document.createElement('div');
    b.style.cssText = 'position:absolute;left:' + dx + 'px;top:' + dy + 'px;width:' + it.width + 'px;height:' + it.height + 'px;border:2px solid rgba(255,0,0,0.8);box-sizing:border-box;pointer-events:none;';
    var l = document.createElement('div');
    l.textContent = String(it.number);
    var labelAnchor = useDocumentSpaceLabels ? dy : it.y;
    var labelTop = labelAnchor < 14 ? '2px' : '-14px';
    l.style.cssText = 'position:absolute;top:' + labelTop + ';left:-2px;background:rgba(255,0,0,0.9);color:#fff;font:bold 11px/14px monospace;padding:0 4px;border-radius:2px;white-space:nowrap;';
    b.appendChild(l);
    c.appendChild(b);
  }
  document.documentElement.appendChild(c);
  return true;
})()
