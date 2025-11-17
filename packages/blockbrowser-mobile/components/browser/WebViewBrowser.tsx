import React, { useRef, useEffect, useCallback } from 'react';
import { View, StyleSheet, Platform } from 'react-native';
import WebView, { WebViewNavigation, WebViewMessageEvent } from 'react-native-webview';
import { useBrowserStore } from '@/store/browser-store';

interface WebViewBrowserProps {
  tabId: string;
  url: string;
  onNavigationChange?: (navState: WebViewNavigation) => void;
}

const INJECTED_JAVASCRIPT = `
(function() {
  // Post page information to React Native
  function sendPageInfo() {
    try {
      const pageInfo = {
        type: 'PAGE_INFO',
        title: document.title,
        url: window.location.href,
        favicon: document.querySelector('link[rel~="icon"]')?.href ||
                 document.querySelector('link[rel~="shortcut icon"]')?.href || '',
      };
      window.ReactNativeWebView.postMessage(JSON.stringify(pageInfo));
    } catch (e) {
      console.error('Error sending page info:', e);
    }
  }

  // Send page info on load
  if (document.readyState === 'complete') {
    sendPageInfo();
  } else {
    window.addEventListener('load', sendPageInfo);
  }

  // Send on title change
  const titleObserver = new MutationObserver(sendPageInfo);
  titleObserver.observe(document.querySelector('title') || document.head, {
    childList: true,
    subtree: true,
    characterData: true,
  });

  // Listen for navigation
  window.addEventListener('beforeunload', () => {
    window.ReactNativeWebView.postMessage(JSON.stringify({
      type: 'NAVIGATION',
      url: window.location.href,
    }));
  });

  // Block popups
  window.open = function() {
    window.ReactNativeWebView.postMessage(JSON.stringify({
      type: 'POPUP_BLOCKED',
      message: 'Popup blocked'
    }));
    return null;
  };
})();
true;
`;

export const WebViewBrowser: React.FC<WebViewBrowserProps> = ({
  tabId,
  url,
  onNavigationChange,
}) => {
  const webViewRef = useRef<WebView>(null);
  const { updateTab, addToHistory } = useBrowserStore();

  const handleNavigationStateChange = useCallback(
    (navState: WebViewNavigation) => {
      updateTab(tabId, {
        url: navState.url,
        title: navState.title || 'Loading...',
        canGoBack: navState.canGoBack,
        canGoForward: navState.canGoForward,
        isLoading: navState.loading,
      });

      if (onNavigationChange) {
        onNavigationChange(navState);
      }

      // Add to history when page loads
      if (!navState.loading && navState.url && navState.title) {
        addToHistory(navState.url, navState.title);
      }
    },
    [tabId, updateTab, onNavigationChange, addToHistory]
  );

  const handleMessage = useCallback(
    (event: WebViewMessageEvent) => {
      try {
        const data = JSON.parse(event.nativeEvent.data);

        switch (data.type) {
          case 'PAGE_INFO':
            updateTab(tabId, {
              title: data.title || 'Untitled',
              favicon: data.favicon,
            });
            break;

          case 'POPUP_BLOCKED':
            console.log('Popup blocked:', data.message);
            break;

          default:
            console.log('Unknown message type:', data.type);
        }
      } catch (error) {
        console.error('Error parsing WebView message:', error);
      }
    },
    [tabId, updateTab]
  );

  const handleLoadProgress = useCallback(
    ({ nativeEvent }: { nativeEvent: { progress: number } }) => {
      updateTab(tabId, {
        progress: nativeEvent.progress,
      });
    },
    [tabId, updateTab]
  );

  const handleError = useCallback(
    (syntheticEvent: any) => {
      const { nativeEvent } = syntheticEvent;
      console.error('WebView error:', nativeEvent);

      updateTab(tabId, {
        isLoading: false,
        title: 'Error loading page',
      });
    },
    [tabId, updateTab]
  );

  return (
    <View style={styles.container}>
      <WebView
        ref={webViewRef}
        source={{ uri: url }}
        style={styles.webview}
        onNavigationStateChange={handleNavigationStateChange}
        onMessage={handleMessage}
        onLoadProgress={handleLoadProgress}
        onError={handleError}
        injectedJavaScript={INJECTED_JAVASCRIPT}
        javaScriptEnabled={true}
        domStorageEnabled={true}
        startInLoadingState={true}
        scalesPageToFit={true}
        allowsBackForwardNavigationGestures={Platform.OS === 'ios'}
        cacheEnabled={true}
        incognito={false}
        thirdPartyCookiesEnabled={true}
        sharedCookiesEnabled={true}
        mediaPlaybackRequiresUserAction={false}
        allowsInlineMediaPlayback={true}
        setSupportMultipleWindows={false}
        allowFileAccess={false}
        allowUniversalAccessFromFileURLs={false}
        mixedContentMode="compatibility"
        userAgent={`Mozilla/5.0 (${Platform.OS === 'ios' ? 'iPhone' : 'Linux; Android 10'}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36 BlockBrowser/1.0`}
      />
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#ffffff',
  },
  webview: {
    flex: 1,
  },
});
