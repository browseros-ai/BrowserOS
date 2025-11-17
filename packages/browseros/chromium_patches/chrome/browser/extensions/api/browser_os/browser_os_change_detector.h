diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_change_detector.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_change_detector.h
new file mode 100644
index 0000000000000..b3287913fd5ac
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_change_detector.h
@@ -0,0 +1,108 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CHANGE_DETECTOR_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CHANGE_DETECTOR_H_
+
+#include <functional>
+
+#include "base/functional/callback.h"
+#include "base/memory/weak_ptr.h"
+#include "base/time/time.h"
+#include "base/timer/timer.h"
+#include "content/public/browser/web_contents_observer.h"
+
+namespace content {
+class WebContents;
+}  // namespace content
+
+namespace ui {
+struct AXUpdatesAndEvents;
+}  // namespace ui
+
+namespace extensions {
+namespace api {
+
+// Change detector that monitors if any change occurred in the web content
+// after an action is performed. This is used to verify that actions like
+// click, type, clear, etc. actually had an effect on the page.
+class BlockBrowserChangeDetector : public content::WebContentsObserver {
+ public:
+  // Execute an action and detect if it causes any change in the page
+  // Returns true if any change was detected within the timeout period
+  static bool ExecuteWithDetection(
+      content::WebContents* web_contents,
+      std::function<void()> action,
+      base::TimeDelta timeout = base::Milliseconds(300));
+
+  // Alternative async version that doesn't block
+  static void ExecuteWithDetectionAsync(
+      content::WebContents* web_contents,
+      std::function<void()> action,
+      base::OnceCallback<void(bool)> callback,
+      base::TimeDelta timeout = base::Milliseconds(300));
+
+  // Constructor and destructor are public for use by factory methods
+  explicit BlockBrowserChangeDetector(content::WebContents* web_contents);
+  ~BlockBrowserChangeDetector() override;
+
+ private:
+  BlockBrowserChangeDetector(const BlockBrowserChangeDetector&) = delete;
+  BlockBrowserChangeDetector& operator=(const BlockBrowserChangeDetector&) = delete;
+
+  // Start monitoring for changes
+  void StartMonitoring();
+
+  // Execute the action and wait for changes
+  bool ExecuteAndWait(std::function<void()> action, base::TimeDelta timeout);
+
+  // Execute the action and notify via callback
+  void ExecuteAndNotify(std::function<void()> action,
+                        base::OnceCallback<void(bool)> callback,
+                        base::TimeDelta timeout);
+
+  // WebContentsObserver overrides - we monitor any of these as "changes"
+  void AccessibilityEventReceived(
+      const ui::AXUpdatesAndEvents& details) override;
+  void DidFinishNavigation(
+      content::NavigationHandle* navigation_handle) override;
+  void DOMContentLoaded(
+      content::RenderFrameHost* render_frame_host) override;
+  void OnFocusChangedInPage(
+      content::FocusedNodeDetails* details) override;
+  void DidOpenRequestedURL(
+      content::WebContents* new_contents,
+      content::RenderFrameHost* source_render_frame_host,
+      const GURL& url,
+      const content::Referrer& referrer,
+      WindowOpenDisposition disposition,
+      ui::PageTransition transition,
+      bool started_from_context_menu,
+      bool renderer_initiated) override;
+
+  // Called when any change is detected
+  void OnChangeDetected();
+
+  // Called when timeout expires
+  void OnTimeout();
+
+  // Simple state tracking
+  bool monitoring_ = false;
+  bool change_detected_ = false;
+  
+  // Callbacks
+  base::OnceClosure wait_callback_;
+  base::OnceCallback<void(bool)> result_callback_;
+  
+  // Timer for timeout
+  base::OneShotTimer timeout_timer_;
+  
+  // Weak pointer factory
+  base::WeakPtrFactory<BlockBrowserChangeDetector> weak_factory_{this};
+};
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CHANGE_DETECTOR_H_
\ No newline at end of file
