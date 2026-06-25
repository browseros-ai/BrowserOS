import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

// Finds elements that behave as interactive but carry no ARIA role (cursor:pointer divs, onclick
// handlers, tabindex, contenteditable) — the SPA pattern the accessibility tree misses. Tags each
// match with a temporary attribute so its backendNodeId can be recovered, then cleans up.
const CURSOR_SCAN_JS = `(function(){
  var interactiveTags=new Set(['a','button','input','select','textarea','details','summary']);
  var interactiveRoles=new Set(['button','link','textbox','checkbox','radio','combobox','listbox',
    'menuitem','menuitemcheckbox','menuitemradio','option','searchbox','slider','spinbutton','switch','tab','treeitem']);
  var out=[];
  var all=document.body?document.body.querySelectorAll('*'):[];
  for(var i=0;i<all.length;i++){
    var el=all[i];
    if(interactiveTags.has(el.tagName.toLowerCase()))continue;
    var role=el.getAttribute('role');
    if(role&&interactiveRoles.has(role.toLowerCase()))continue;
    var style=getComputedStyle(el);
    var hasCursor=style.cursor==='pointer';
    var hasOnClick=el.hasAttribute('onclick')||el.onclick!==null;
    var tabIdx=el.getAttribute('tabindex');
    var hasTabIndex=tabIdx!==null&&tabIdx!=='-1';
    var editable=el.isContentEditable;
    if(!hasCursor&&!hasOnClick&&!hasTabIndex&&!editable)continue;
    if(hasCursor&&!hasOnClick&&!hasTabIndex&&!editable){
      var p=el.parentElement;
      if(p&&getComputedStyle(p).cursor==='pointer')continue;
    }
    var rect=el.getBoundingClientRect();
    if(rect.width===0||rect.height===0)continue;
    el.setAttribute('data-__bcid',String(i));
    var reasons=[];
    if(hasCursor)reasons.push('cursor:pointer');
    if(hasOnClick)reasons.push('onclick');
    if(hasTabIndex)reasons.push('tabindex');
    if(editable)reasons.push('contenteditable');
    out.push({marker:String(i),reasons:reasons});
  }
  return out;
})()`

interface ScanHit {
  marker: string
  reasons: string[]
}

/** backendNodeId → reasons for cursor-interactive elements in this frame. Best-effort. */
export async function findCursorHits(
  session: ProtocolApi,
): Promise<Map<number, string[]>> {
  const hits = new Map<number, string[]>()

  let found: ScanHit[] | undefined
  try {
    const result = await session.Runtime.evaluate({
      expression: CURSOR_SCAN_JS,
      returnByValue: true,
    })
    found = result.result?.value as ScanHit[] | undefined
  } catch {
    return hits
  }
  if (!found?.length) return hits

  for (const hit of found) {
    try {
      const query = await session.Runtime.evaluate({
        expression: `document.querySelector('[data-__bcid="${hit.marker}"]')`,
        returnByValue: false,
      })
      const objectId = query.result?.objectId
      if (!objectId) continue
      const described = await session.DOM.describeNode({ objectId })
      const backendNodeId = described.node?.backendNodeId
      if (backendNodeId !== undefined) hits.set(backendNodeId, hit.reasons)
    } catch {
      // element vanished between scan and resolve
    }
  }

  await session.Runtime.evaluate({
    expression:
      "document.querySelectorAll('[data-__bcid]').forEach(function(e){e.removeAttribute('data-__bcid')})",
    returnByValue: true,
  }).catch(() => {})

  return hits
}
