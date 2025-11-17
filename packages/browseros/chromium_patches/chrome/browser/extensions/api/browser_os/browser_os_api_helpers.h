diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.h
new file mode 100644
index 0000000000000..1b12c66835998
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.h
@@ -0,0 +1,136 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_HELPERS_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_HELPERS_H_
+
+#include <string>
+
+#include "base/functional/callback.h"
+#include "ui/gfx/geometry/point_f.h"
+
+namespace content {
+class WebContents;
+class RenderWidgetHost;
+}  // namespace content
+
+namespace extensions {
+namespace api {
+
+struct NodeInfo;
+
+// Returns the multiplicative factor that converts CSS pixels (frame
+// coordinates) to widget DIPs for input events. This matches DevTools'
+// InputHandler::ScaleFactor(): browser zoom × CSS zoom × page scale. The
+// device scale factor (DSF) is NOT included because compositor handles it and
+// input expects widget DIPs (we also set screen = widget).
+float CssToWidgetScale(content::WebContents* web_contents,
+                       content::RenderWidgetHost* rwh);
+
+// Returns the center point of a node's bounds.
+// Bounds are already in CSS pixels from SnapshotProcessor.
+gfx::PointF GetNodeCenterPoint(content::WebContents* web_contents,
+                               const NodeInfo& node_info);
+
+// Helper to create and dispatch mouse events for clicking
+void PointClick(content::WebContents* web_contents, 
+                  const gfx::PointF& point);
+
+// Helper to perform HTML-based click using JS (uses ID, class, or tag)
+void HtmlClick(content::WebContents* web_contents,
+                      const NodeInfo& node_info);
+
+// Helper to perform HTML-based focus using JS (uses ID, class, or tag)
+void HtmlFocus(content::WebContents* web_contents,
+                      const NodeInfo& node_info);
+
+// Helper to perform accessibility action: DoDefault (click)
+// Returns true if action was sent successfully
+bool AccessibilityDoDefault(content::WebContents* web_contents,
+                            const NodeInfo& node_info);
+
+// Helper to perform accessibility action: Focus
+// Returns true if action was sent successfully
+bool AccessibilityFocus(content::WebContents* web_contents,
+                       const NodeInfo& node_info);
+
+// Helper to perform accessibility action: ScrollToMakeVisible
+// center_in_viewport: if true, centers element in viewport; otherwise uses closest edge
+// Returns true if action was sent successfully
+bool AccessibilityScrollToMakeVisible(content::WebContents* web_contents,
+                                      const NodeInfo& node_info,
+                                      bool center_in_viewport = true);
+
+// Helper to perform accessibility action: SetValue
+// Sets the value of an input field or editable element
+// Returns true if action was sent successfully
+bool AccessibilitySetValue(content::WebContents* web_contents,
+                           const NodeInfo& node_info,
+                           const std::string& text);
+
+// Helper to perform scroll actions using mouse wheel events
+void Scroll(content::WebContents* web_contents,
+                   int delta_x,
+                   int delta_y,
+                   bool precise = false);
+
+// Helper to send special key events
+void KeyPress(content::WebContents* web_contents,
+                    const std::string& key);
+
+// Helper to type text into a focused element using native IME
+void NativeType(content::WebContents* web_contents,
+                const std::string& text);
+
+// Helper to set text value using JavaScript
+void JavaScriptType(content::WebContents* web_contents,
+                    const NodeInfo& node_info,
+                    const std::string& text);
+
+// Helper to perform a click with change detection and retrying
+// Returns true if the click caused a change in the page
+bool ClickWithDetection(content::WebContents* web_contents,
+                        const NodeInfo& node_info);
+
+// Helper to perform typing with change detection
+// Returns true if the typing caused a change in the page
+bool TypeWithDetection(content::WebContents* web_contents,
+                      const NodeInfo& node_info,
+                      const std::string& text);
+
+// Helper to clear an input field with change detection
+// Returns true if the clear caused a change in the page
+bool ClearWithDetection(content::WebContents* web_contents,
+                       const NodeInfo& node_info);
+
+// Helper to send a key press with change detection
+// Returns true if the key press caused a change in the page
+bool KeyPressWithDetection(content::WebContents* web_contents,
+                          const std::string& key);
+
+// Helper to show highlights for clickable, typeable, and selectable elements that are in viewport
+// Only highlights elements that are actually visible and interactable
+void ShowHighlights(content::WebContents* web_contents,
+                                 const std::unordered_map<uint32_t, NodeInfo>& node_mappings,
+                                 bool show_labels = true);
+
+// Helper to remove all bounding box highlights from the page
+void RemoveHighlights(content::WebContents* web_contents);
+
+// Helper to click at specific coordinates with change detection
+// Returns true if the click caused a detectable change in the page
+bool ClickCoordinatesWithDetection(content::WebContents* web_contents,
+                                   const gfx::PointF& point);
+
+// Helper to type text after clicking at coordinates to focus element
+// First clicks at the coordinates to focus an element, then types the text
+// Returns true if the operation succeeded
+bool TypeAtCoordinatesWithDetection(content::WebContents* web_contents,
+                                    const gfx::PointF& point,
+                                    const std::string& text);
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_HELPERS_H_
