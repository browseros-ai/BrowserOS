import React, { useRef, useImperativeHandle, forwardRef, useCallback } from 'react';
import { View, StyleSheet, Platform, Alert } from 'react-native';
import WebView, { WebViewNavigation, WebViewMessageEvent } from 'react-native-webview';
import Clipboard from '@react-native-clipboard/clipboard';
import { useBrowserStore } from '@/store/browser-store';
import { useSettingsStore } from '@/store/settings-store';

interface WebViewBrowserProps {
  tabId: string;
  url: string;
  isIncognito?: boolean;
  onNavigationChange?: (navState: WebViewNavigation) => void;
}

export interface WebViewBrowserRef {
  goBack: () => void;
  goForward: () => void;
  reload: () => void;
  stopLoading: () => void;
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
                 document.querySelector('link[rel~="shortcut icon"]')?.href ||
                 window.location.origin + '/favicon.ico',
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
  const titleElement = document.querySelector('title');
  if (titleElement) {
    titleObserver.observe(titleElement, {
      childList: true,
      subtree: true,
      characterData: true,
    });
  }

  // Listen for navigation
  window.addEventListener('beforeunload', () => {
    window.ReactNativeWebView.postMessage(JSON.stringify({
      type: 'NAVIGATION',
      url: window.location.href,
    }));
  });

  // Block popups
  const originalWindowOpen = window.open;
  window.open = function(...args) {
    window.ReactNativeWebView.postMessage(JSON.stringify({
      type: 'POPUP_BLOCKED',
      url: args[0],
    }));
    return null;
  };

  // Handle downloads
  document.addEventListener('click', function(e) {
    const target = e.target;
    if (target.tagName === 'A' && target.hasAttribute('download')) {
      e.preventDefault();
      window.ReactNativeWebView.postMessage(JSON.stringify({
        type: 'DOWNLOAD',
        url: target.href,
        filename: target.getAttribute('download'),
      }));
    }
  });

  // Context menu support
  let longPressTimer;
  let pressTarget;

  document.addEventListener('touchstart', function(e) {
    pressTarget = e.target;
    longPressTimer = setTimeout(() => {
      const target = pressTarget;
      if (target.tagName === 'A') {
        window.ReactNativeWebView.postMessage(JSON.stringify({
          type: 'LONG_PRESS_LINK',
          url: target.href,
          text: target.innerText || target.textContent,
        }));
      } else if (target.tagName === 'IMG') {
        window.ReactNativeWebView.postMessage(JSON.stringify({
          type: 'LONG_PRESS_IMAGE',
          url: target.src,
          alt: target.alt,
        }));
      }
    }, 500);
  }, { passive: true });

  document.addEventListener('touchend', function() {
    clearTimeout(longPressTimer);
  }, { passive: true });

  document.addEventListener('touchmove', function() {
    clearTimeout(longPressTimer);
  }, { passive: true });
})();
true;
`;

export const WebViewBrowser = forwardRef<WebViewBrowserRef, WebViewBrowserProps>(
  ({ tabId, url, isIncognito = false, onNavigationChange }, ref) => {
    const webViewRef = useRef<WebView>(null);
    const { updateTab, addToHistory, addTab, setActiveTab } = useBrowserStore();
    const { enableJavaScript } = useSettingsStore();

    // Expose methods to parent component
    useImperativeHandle(ref, () => ({
      goBack: () => webViewRef.current?.goBack(),
      goForward: () => webViewRef.current?.goForward(),
      reload: () => webViewRef.current?.reload(),
      stopLoading: () => webViewRef.current?.stopLoading(),
    }));

    const handleNavigationStateChange = useCallback(
      (navState: WebViewNavigation) => {
        updateTab(tabId, {
          url: navState.url,
          title: navState.title || 'Loading...',
          canGoBack: navState.canGoBack,
          canGoForward: navState.canGoForward,
          isLoading: navState.loading,
        });

        onNavigationChange?.(navState);

        // Add to history when page loads (not in incognito)
        if (!navState.loading && navState.url && navState.title && !isIncognito) {
          addToHistory(navState.url, navState.title);
        }
      },
      [tabId, updateTab, onNavigationChange, addToHistory, isIncognito]
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
              Alert.alert('Popup Blocked', `Blocked popup: ${data.url}`);
              break;

            case 'DOWNLOAD':
              Alert.alert(
                'Download',
                `Download requested: ${data.filename || data.url}`,
                [
                  { text: 'Cancel', style: 'cancel' },
                  {
                    text: 'Open',
                    onPress: () => {
                      const newTabId = addTab(data.url);
                      setActiveTab(newTabId);
                    },
                  },
                ]
              );
              break;

            case 'LONG_PRESS_LINK':
              Alert.alert(
                'Link Options',
                data.url,
                [
                  { text: 'Cancel', style: 'cancel' },
                  {
                    text: 'Open in New Tab',
                    onPress: () => {
                      const newTabId = addTab(data.url);
                      setActiveTab(newTabId);
                      Alert.alert('Success', 'Link opened in new tab');
                    },
                  },
                  {
                    text: 'Open in Background',
                    onPress: () => {
                      addTab(data.url);
                      Alert.alert('Success', 'Tab opened in background');
                    },
                  },
                  {
                    text: 'Copy Link',
                    onPress: () => {
                      Clipboard.setString(data.url);
                      Alert.alert('Copied', 'Link copied to clipboard');
                    },
                  },
                ]
              );
              break;

            case 'LONG_PRESS_IMAGE':
              Alert.alert(
                'Image Options',
                data.url,
                [
                  { text: 'Cancel', style: 'cancel' },
                  {
                    text: 'Open Image',
                    onPress: () => {
                      const newTabId = addTab(data.url);
                      setActiveTab(newTabId);
                    },
                  },
                  {
                    text: 'Copy Image URL',
                    onPress: () => {
                      Clipboard.setString(data.url);
                      Alert.alert('Copied', 'Image URL copied to clipboard');
                    },
                  },
                ]
              );
              break;

            default:
              console.log('blockbrowser: Unknown message type:', data.type);
          }
        } catch (error) {
          console.error('blockbrowser: Error parsing WebView message:', error);
        }
      },
      [tabId, updateTab, addTab, setActiveTab]
    );

    const handleLoadProgress = useCallback(
      ({ nativeEvent }: { nativeEvent: { progress: number } }) => {
        updateTab(tabId, {
          progress: nativeEvent.progress,
        });
      },
      [tabId, updateTab]
    );

    const handleLoadStart = useCallback(() => {
      updateTab(tabId, {
        isLoading: true,
        progress: 0,
      });
    }, [tabId, updateTab]);

    const handleLoadEnd = useCallback(() => {
      updateTab(tabId, {
        isLoading: false,
        progress: 1,
      });
    }, [tabId, updateTab]);

    const handleError = useCallback(
      (syntheticEvent: any) => {
        const { nativeEvent } = syntheticEvent;
        console.error('blockbrowser: WebView error:', nativeEvent);

        updateTab(tabId, {
          isLoading: false,
          title: 'Error loading page',
        });

        Alert.alert(
          'Page Load Error',
          `Failed to load ${nativeEvent.url}\n\n${nativeEvent.description || 'Unknown error'}`,
          [
            { text: 'OK' },
            {
              text: 'Retry',
              onPress: () => webViewRef.current?.reload(),
            },
          ]
        );
      },
      [tabId, updateTab]
    );

    const handleHttpError = useCallback(
      (syntheticEvent: any) => {
        const { nativeEvent } = syntheticEvent;
        console.warn('blockbrowser: HTTP error:', nativeEvent.statusCode, nativeEvent.url);
      },
      []
    );

    // Generate user agent
    const userAgent = `Mozilla/5.0 (${
      Platform.OS === 'ios' ? 'iPhone; CPU iPhone OS 16_0 like Mac OS X' : 'Linux; Android 13'
    }) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36 BlockBrowser/1.0`;

    return (
      <View style={styles.container}>
        <WebView
          ref={webViewRef}
          source={{ uri: url }}
          style={styles.webview}
          onNavigationStateChange={handleNavigationStateChange}
          onMessage={handleMessage}
          onLoadProgress={handleLoadProgress}
          onLoadStart={handleLoadStart}
          onLoadEnd={handleLoadEnd}
          onError={handleError}
          onHttpError={handleHttpError}
          injectedJavaScript={INJECTED_JAVASCRIPT}
          javaScriptEnabled={enableJavaScript}
          domStorageEnabled={true}
          startInLoadingState={true}
          scalesPageToFit={true}
          allowsBackForwardNavigationGestures={Platform.OS === 'ios'}
          cacheEnabled={!isIncognito}
          incognito={isIncognito}
          thirdPartyCookiesEnabled={!isIncognito}
          sharedCookiesEnabled={!isIncognito}
          mediaPlaybackRequiresUserAction={false}
          allowsInlineMediaPlayback={true}
          setSupportMultipleWindows={false}
          allowFileAccess={false}
          allowUniversalAccessFromFileURLs={false}
          mixedContentMode="compatibility"
          userAgent={userAgent}
          originWhitelist={['*']}
          androidHardwareAccelerationDisabled={false}
          androidLayerType="hardware"
        />
      </View>
    );
  }
);

WebViewBrowser.displayName = 'WebViewBrowser';

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#ffffff',
  },
  webview: {
    flex: 1,
  },
});
