/* deepscan-disable UNUSED_EXPR */
// CDP Runtime.callFunctionOn consumes the final expression as its functionDeclaration; it must remain uninvoked in this source asset.
// biome-ignore-all lint: Injected ES5 asset mirrors the TypeScript browser-core runtime string.
// biome-ignore format: Keep injected script byte-oriented and close to the TypeScript source.
(function(markerAttribute,markerToken){
  var document=this;
  if(document.querySelector('['+markerAttribute+']'))return{collision:true,candidates:[]};
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
    el.setAttribute(markerAttribute,markerToken+':'+String(i));
    var reasons=[];
    if(hasCursor)reasons.push('cursor:pointer');
    if(hasOnClick)reasons.push('onclick');
    if(hasTabIndex)reasons.push('tabindex');
    if(editable)reasons.push('contenteditable');
    out.push({marker:String(i),reasons:reasons});
  }
  return{collision:false,candidates:out};
})
