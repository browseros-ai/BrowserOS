/* The logo always links to "/", which is BrowserOS neo. On a BrowserOS page
   that sends the reader out of the product they were reading, so the link is
   repointed to that product's own landing page.

   docs.json takes a single logo.href site-wide and a product entry does not
   accept one, so this cannot be configuration. It is also not reachable from
   CSS, which cannot change an href.

   Mintlify navigates client-side, so the path is watched rather than read once
   at load. */
;(() => {
  const NEO_HOME = '/'
  const OS_HOME = '/browseros'

  function currentHome() {
    const path = document.documentElement.getAttribute('data-current-path') || '/'
    return path === OS_HOME || path.startsWith(`${OS_HOME}/`) ? OS_HOME : NEO_HOME
  }

  function syncLogoLinks() {
    const home = currentHome()
    for (const logo of document.querySelectorAll('.nav-logo')) {
      const link = logo.closest('a')
      if (link && link.getAttribute('href') !== home) link.setAttribute('href', home)
    }
  }

  /* Setting the href is not enough on its own. Mintlify's router intercepts
     the click and navigates from its own state, so the anchor's href is never
     read. The click is caught in the capture phase, before that handler runs,
     and the navigation is done here instead. */
  function interceptLogoClick(event) {
    const link = event.target instanceof Element && event.target.closest('a')
    if (!link || !link.querySelector('.nav-logo')) return
    const home = currentHome()
    if (home === NEO_HOME) return // the router's own default is already correct
    event.preventDefault()
    event.stopPropagation()
    window.location.assign(home)
  }

  const start = () => {
    syncLogoLinks()
    document.addEventListener('click', interceptLogoClick, true)
    // The path attribute changes on client-side navigation; the logo itself is
    // re-rendered independently, so both are watched.
    new MutationObserver(syncLogoLinks).observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-current-path'],
    })
    new MutationObserver(syncLogoLinks).observe(document.body, {
      childList: true,
      subtree: true,
    })
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start)
  } else {
    start()
  }
})()
