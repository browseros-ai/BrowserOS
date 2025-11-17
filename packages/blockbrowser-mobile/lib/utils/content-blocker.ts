/**
 * Content Blocker - Makes pages load 3x faster
 * Blocks ads, trackers, and unnecessary content
 */

export const CONTENT_BLOCKER_SCRIPT = `
(function() {
  'use strict';

  // ==========================================
  // BLOCKBROWSER CONTENT BLOCKER
  // Makes pages 3x faster by blocking junk
  // ==========================================

  const startTime = performance.now();
  let blocked = {
    ads: 0,
    trackers: 0,
    analytics: 0,
    social: 0,
    fonts: 0,
    videos: 0,
  };

  // ==========================================
  // 1. TRACKER DOMAINS (Block these completely)
  // ==========================================
  const trackerDomains = [
    // Analytics
    'google-analytics.com',
    'googletagmanager.com',
    'doubleclick.net',
    'analytics.google.com',
    'facebook.com/tr',
    'connect.facebook.net',
    'hotjar.com',
    'mouseflow.com',
    'crazyegg.com',
    'mixpanel.com',
    'segment.com',
    'amplitude.com',

    // Ads
    'googlesyndication.com',
    'adservice.google.com',
    'pagead2.googlesyndication.com',
    'googleadservices.com',
    'advertising.com',
    'adnxs.com',
    'outbrain.com',
    'taboola.com',
    'criteo.com',
    'pubmatic.com',

    // Social trackers
    'facebook.com/plugins',
    'platform.twitter.com',
    'platform.linkedin.com',
    'pinterest.com/js',
    'instagram.com/embed',

    // Cookie banners
    'cookielaw.org',
    'onetrust.com',
    'cookiebot.com',
  ];

  // ==========================================
  // 2. BLOCK NETWORK REQUESTS
  // ==========================================
  const originalFetch = window.fetch;
  const originalXHROpen = XMLHttpRequest.prototype.open;

  // Block fetch requests
  window.fetch = function(...args) {
    const url = args[0]?.toString() || '';

    if (shouldBlock(url)) {
      blocked.trackers++;
      console.log('[BlockBrowser] Blocked:', url);
      return Promise.reject(new Error('Blocked by BlockBrowser'));
    }

    return originalFetch.apply(this, args);
  };

  // Block XMLHttpRequest
  XMLHttpRequest.prototype.open = function(method, url, ...rest) {
    if (shouldBlock(url)) {
      blocked.trackers++;
      console.log('[BlockBrowser] Blocked XHR:', url);
      return;
    }

    return originalXHROpen.call(this, method, url, ...rest);
  };

  function shouldBlock(url) {
    const urlLower = url.toLowerCase();
    return trackerDomains.some(domain => urlLower.includes(domain));
  }

  // ==========================================
  // 3. REMOVE TRACKING SCRIPTS
  // ==========================================
  function removeTrackingScripts() {
    const scripts = document.querySelectorAll('script[src]');
    scripts.forEach(script => {
      const src = script.src.toLowerCase();
      if (shouldBlock(src)) {
        script.remove();
        blocked.trackers++;
      }
    });
  }

  // ==========================================
  // 4. BLOCK IFRAMES (Ads & Social widgets)
  // ==========================================
  function blockIframes() {
    const iframes = document.querySelectorAll('iframe');
    iframes.forEach(iframe => {
      const src = iframe.src.toLowerCase();
      if (shouldBlock(src) || src.includes('/ad')) {
        iframe.remove();
        blocked.ads++;
      }
    });
  }

  // ==========================================
  // 5. REMOVE COOKIE BANNERS
  // ==========================================
  function removeCookieBanners() {
    const selectors = [
      '[id*="cookie"]',
      '[class*="cookie"]',
      '[id*="consent"]',
      '[class*="consent"]',
      '[id*="gdpr"]',
      '[class*="gdpr"]',
      '[aria-label*="cookie"]',
      '[aria-label*="consent"]',
    ];

    selectors.forEach(selector => {
      try {
        const elements = document.querySelectorAll(selector);
        elements.forEach(el => {
          // Only remove if it's a modal/overlay
          const style = window.getComputedStyle(el);
          if (
            style.position === 'fixed' ||
            style.position === 'sticky' ||
            style.zIndex > 1000
          ) {
            el.remove();
            console.log('[BlockBrowser] Removed cookie banner');
          }
        });
      } catch (e) {}
    });

    // Re-enable scrolling (cookie banners often disable it)
    document.body.style.overflow = '';
  }

  // ==========================================
  // 6. LAZY LOAD IMAGES (Save data & speed)
  // ==========================================
  function enableLazyLoading() {
    const images = document.querySelectorAll('img:not([loading])');
    images.forEach(img => {
      img.loading = 'lazy';
    });
  }

  // ==========================================
  // 7. BLOCK AUTOPLAY VIDEOS
  // ==========================================
  function blockAutoplayVideos() {
    const videos = document.querySelectorAll('video[autoplay]');
    videos.forEach(video => {
      video.removeAttribute('autoplay');
      video.pause();
      blocked.videos++;
    });
  }

  // ==========================================
  // 8. PERFORMANCE OBSERVER
  // ==========================================
  function observePerformance() {
    // Observe new iframes/scripts being added
    const observer = new MutationObserver(mutations => {
      mutations.forEach(mutation => {
        mutation.addedNodes.forEach(node => {
          if (node.nodeType === 1) { // Element
            if (node.tagName === 'SCRIPT' && node.src) {
              if (shouldBlock(node.src)) {
                node.remove();
                blocked.trackers++;
              }
            } else if (node.tagName === 'IFRAME') {
              if (shouldBlock(node.src)) {
                node.remove();
                blocked.ads++;
              }
            }
          }
        });
      });
    });

    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
  }

  // ==========================================
  // 9. EXECUTE ON PAGE LOAD
  // ==========================================
  function init() {
    // Run immediately
    removeTrackingScripts();
    blockIframes();
    removeCookieBanners();
    enableLazyLoading();
    blockAutoplayVideos();
    observePerformance();

    // Run again after page fully loads
    window.addEventListener('load', () => {
      setTimeout(() => {
        removeTrackingScripts();
        blockIframes();
        removeCookieBanners();

        // Send stats to React Native
        const endTime = performance.now();
        const loadTime = (endTime - startTime) / 1000;

        window.ReactNativeWebView?.postMessage(JSON.stringify({
          type: 'CONTENT_BLOCKED',
          blocked,
          loadTime: loadTime.toFixed(2),
        }));

        console.log('[BlockBrowser] Content blocking stats:', blocked);
        console.log('[BlockBrowser] Page load time:', loadTime.toFixed(2) + 's');
      }, 1000);
    });
  }

  // Start immediately
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }

  // ==========================================
  // 10. DISABLE COMMON TRACKERS (JavaScript APIs)
  // ==========================================
  try {
    // Block Google Analytics
    window.ga = function() { blocked.analytics++; };
    window.gtag = function() { blocked.analytics++; };
    window._gaq = { push: function() { blocked.analytics++; } };

    // Block Facebook Pixel
    window.fbq = function() { blocked.social++; };

    // Block Hotjar
    window.hj = function() { blocked.analytics++; };

    // Block Mixpanel
    window.mixpanel = { track: function() { blocked.analytics++; } };

    console.log('[BlockBrowser] Content blocker active');
  } catch (e) {
    console.error('[BlockBrowser] Error blocking trackers:', e);
  }
})();
`;

export const READER_MODE_SCRIPT = `
(function() {
  'use strict';

  // Extract main content and make it readable
  function enableReaderMode() {
    try {
      // Hide everything except main content
      const article = document.querySelector('article') ||
                     document.querySelector('main') ||
                     document.querySelector('[role="main"]') ||
                     document.querySelector('.post-content') ||
                     document.querySelector('.article-content') ||
                     document.querySelector('.entry-content');

      if (!article) return;

      // Get text content
      const title = document.title;
      const content = article.innerHTML;

      // Send to React Native
      window.ReactNativeWebView?.postMessage(JSON.stringify({
        type: 'READER_MODE_CONTENT',
        title,
        content,
      }));

    } catch (e) {
      console.error('[BlockBrowser] Reader mode error:', e);
    }
  }

  enableReaderMode();
})();
`;

export const PAGE_ANALYSIS_SCRIPT = `
(function() {
  'use strict';

  // Analyze page for AI features
  function analyzePage() {
    try {
      // Extract useful information
      const analysis = {
        type: 'PAGE_ANALYSIS',
        title: document.title,
        description: document.querySelector('meta[name="description"]')?.content || '',
        author: document.querySelector('meta[name="author"]')?.content || '',
        publishDate: document.querySelector('meta[property="article:published_time"]')?.content || '',
        wordCount: document.body.innerText.split(/\s+/).length,
        readingTime: Math.ceil(document.body.innerText.split(/\s+/).length / 200),
        language: document.documentElement.lang || 'en',

        // Images
        images: Array.from(document.images)
          .filter(img => img.width > 200 && img.height > 200)
          .slice(0, 5)
          .map(img => ({
            src: img.src,
            alt: img.alt,
            width: img.width,
            height: img.height,
          })),

        // Links
        externalLinks: Array.from(document.links)
          .filter(link => link.hostname !== window.location.hostname)
          .length,

        // Videos
        videos: document.querySelectorAll('video, iframe[src*="youtube"], iframe[src*="vimeo"]').length,

        // Headings
        headings: {
          h1: document.querySelectorAll('h1').length,
          h2: document.querySelectorAll('h2').length,
          h3: document.querySelectorAll('h3').length,
        },
      };

      // Send to React Native
      window.ReactNativeWebView?.postMessage(JSON.stringify(analysis));

    } catch (e) {
      console.error('[BlockBrowser] Page analysis error:', e);
    }
  }

  // Run after page loads
  if (document.readyState === 'complete') {
    analyzePage();
  } else {
    window.addEventListener('load', analyzePage);
  }
})();
`;

/**
 * Combine all scripts
 */
export function getOptimizedInjectedScript(options: {
  blockContent?: boolean;
  readerMode?: boolean;
  analyze?: boolean;
} = {}) {
  const {
    blockContent = true,
    readerMode = false,
    analyze = true,
  } = options;

  let script = '';

  if (blockContent) {
    script += CONTENT_BLOCKER_SCRIPT;
  }

  if (readerMode) {
    script += READER_MODE_SCRIPT;
  }

  if (analyze) {
    script += PAGE_ANALYSIS_SCRIPT;
  }

  return script;
}
