diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.cc b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.cc
new file mode 100644
index 0000000000000..f40a2424641e1
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.cc
@@ -0,0 +1,1073 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_api_helpers.h"
+
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/stringprintf.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/task/sequenced_task_runner.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_change_detector.h"
+#include "components/input/native_web_keyboard_event.h"
+#include "content/public/browser/render_frame_host.h"
+#include "content/browser/renderer_host/render_widget_host_impl.h"
+#include "content/public/browser/render_widget_host.h"
+#include "content/public/browser/render_widget_host_view.h"
+#include "content/browser/renderer_host/render_widget_host_view_base.h"
+#include "content/browser/web_contents/web_contents_impl.h"
+#include "content/public/browser/web_contents.h"
+#include "third_party/blink/public/common/input/web_input_event.h"
+#include "third_party/blink/public/common/input/web_keyboard_event.h"
+#include "third_party/blink/public/common/input/web_mouse_event.h"
+#include "third_party/blink/public/common/input/web_mouse_wheel_event.h"
+#include "third_party/blink/public/common/page/page_zoom.h"
+#include "ui/base/ime/ime_text_span.h"
+#include "ui/events/base_event_utils.h"
+#include "ui/events/keycodes/dom/dom_code.h"
+#include "ui/events/keycodes/dom/dom_key.h"
+#include "ui/events/keycodes/keyboard_codes.h"
+#include "ui/gfx/geometry/point_f.h"
+#include "ui/gfx/range/range.h"
+#include "ui/accessibility/ax_action_data.h"
+#include "ui/accessibility/ax_enums.mojom.h"
+
+namespace extensions {
+namespace api {
+
+// Compute CSS->widget scale matching DevTools InputHandler::ScaleFactor.
+// We intentionally exclude device scale factor (DSF). Widget coordinates
+// used by input are in DIPs; DSF is handled by the compositor. We also set
+// PositionInScreen = PositionInWidget to avoid unit mixing on HiDPI.
+float CssToWidgetScale(content::WebContents* web_contents,
+                       content::RenderWidgetHost* rwh) {
+  float zoom = 1.0f;
+  if (auto* rwhi = static_cast<content::RenderWidgetHostImpl*>(rwh)) {
+    if (auto* wci = static_cast<content::WebContentsImpl*>(web_contents)) {
+      zoom = blink::ZoomLevelToZoomFactor(wci->GetPendingZoomLevel(rwhi));
+    }
+  }
+
+  float css_zoom = 1.0f;
+  if (auto* view = rwh ? rwh->GetView() : nullptr) {
+    if (auto* view_base =
+            static_cast<content::RenderWidgetHostViewBase*>(view)) {
+      css_zoom = view_base->GetCSSZoomFactor();
+    }
+  }
+
+  float page_scale = 1.0f;
+  if (auto* wci = static_cast<content::WebContentsImpl*>(web_contents)) {
+    page_scale = wci->GetPrimaryPage().GetPageScaleFactor();
+  }
+
+  return zoom * css_zoom * page_scale;
+}
+
+// Helper function to get center point of a node's bounds.
+// Bounds are already stored in CSS pixels from SnapshotProcessor,
+// so no DSF conversion is needed.
+gfx::PointF GetNodeCenterPoint(content::WebContents* web_contents,
+                               const NodeInfo& node_info) {
+  // Simple calculation - bounds are already in CSS pixels
+  return gfx::PointF(
+      node_info.bounds.x() + node_info.bounds.width() / 2.0f,
+      node_info.bounds.y() + node_info.bounds.height() / 2.0f);
+}
+
+
+// Helper to create and dispatch mouse events for clicking
+void PointClick(content::WebContents* web_contents, 
+                  const gfx::PointF& point) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh)
+    return;
+    
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv)
+    return;
+
+  // The incoming point is in CSS pixels (already normalized by DSF if needed).
+  // Convert CSS → widget DIPs using the same scale chain as DevTools.
+  gfx::PointF css_point = point;
+  const float scale = CssToWidgetScale(web_contents, rwh);
+  gfx::PointF widget_point(css_point.x() * scale, css_point.y() * scale);
+
+  // Create mouse down event
+  blink::WebMouseEvent mouse_down;
+  mouse_down.SetType(blink::WebInputEvent::Type::kMouseDown);
+  mouse_down.button = blink::WebPointerProperties::Button::kLeft;
+  mouse_down.click_count = 1;
+  mouse_down.SetPositionInWidget(widget_point.x(), widget_point.y());
+  // Align with DevTools: screen position equals widget position to avoid
+  // unit-mixing on HiDPI. The compositor handles DSF.
+  mouse_down.SetPositionInScreen(widget_point.x(), widget_point.y());
+  mouse_down.SetTimeStamp(ui::EventTimeForNow());
+  mouse_down.SetModifiers(blink::WebInputEvent::kLeftButtonDown);
+  
+  // Create mouse up event
+  blink::WebMouseEvent mouse_up;
+  mouse_up.SetType(blink::WebInputEvent::Type::kMouseUp);
+  mouse_up.button = blink::WebPointerProperties::Button::kLeft;
+  mouse_up.click_count = 1;
+  mouse_up.SetPositionInWidget(widget_point.x(), widget_point.y());
+  mouse_up.SetPositionInScreen(widget_point.x(), widget_point.y());
+  mouse_up.SetTimeStamp(ui::EventTimeForNow());
+  
+  // Send the events
+  rwh->ForwardMouseEvent(mouse_down);
+  rwh->ForwardMouseEvent(mouse_up);
+}
+
+// Helper to perform HTML-based click using JS (uses ID, class, or tag)
+void HtmlClick(content::WebContents* web_contents,
+                      const NodeInfo& node_info) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  // Build the JavaScript to find and click the element
+  std::u16string js_code = u"(function() {";
+  
+  // Try to find element by ID first
+  auto id_it = node_info.attributes.find("id");
+  if (id_it != node_info.attributes.end() && !id_it->second.empty()) {
+    js_code += u"  var element = document.getElementById('" + 
+               base::UTF8ToUTF16(id_it->second) + u"');";
+    js_code += u"  if (element) {";
+    js_code += u"    element.click();";
+    js_code += u"    return 'clicked by id';";
+    js_code += u"  }";
+  }
+  
+  // Try to find by class and tag combination
+  auto class_it = node_info.attributes.find("class");
+  auto tag_it = node_info.attributes.find("html-tag");
+  
+  if (class_it != node_info.attributes.end() && !class_it->second.empty() &&
+      tag_it != node_info.attributes.end() && !tag_it->second.empty()) {
+    // Split class names and create selector
+    std::string class_selector = "." + class_it->second;
+    // Replace spaces with dots for multiple classes
+    for (size_t i = 0; i < class_selector.length(); ++i) {
+      if (class_selector[i] == ' ') {
+        class_selector[i] = '.';
+      }
+    }
+    
+    js_code += u"  var elements = document.querySelectorAll('" + 
+               base::UTF8ToUTF16(tag_it->second + class_selector) + u"');";
+    js_code += u"  if (elements.length > 0) {";
+    js_code += u"    elements[0].click();";
+    js_code += u"    return 'clicked by class and tag';";
+    js_code += u"  }";
+  }
+  
+  // Fallback: try just by tag name if available
+  if (tag_it != node_info.attributes.end() && !tag_it->second.empty()) {
+    js_code += u"  var elements = document.getElementsByTagName('" + 
+               base::UTF8ToUTF16(tag_it->second) + u"');";
+    js_code += u"  if (elements.length > 0) {";
+    js_code += u"    elements[0].click();";
+    js_code += u"    return 'clicked by tag';";
+    js_code += u"  }";
+  }
+  
+  js_code += u"  return 'no element found';";
+  js_code += u"})();";
+  
+  // Execute the JavaScript
+  rfh->ExecuteJavaScriptForTests(
+      js_code,
+      base::NullCallback(),
+      /*honor_js_content_settings=*/false);
+}
+
+// Helper to perform HTML-based focus using JS (uses ID, class, or tag)
+void HtmlFocus(content::WebContents* web_contents,
+                      const NodeInfo& node_info) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  // Build the JavaScript to find and focus the element
+  std::u16string js_code = u"(function() {";
+  
+  // Try to find element by ID first
+  auto id_it = node_info.attributes.find("id");
+  if (id_it != node_info.attributes.end() && !id_it->second.empty()) {
+    js_code += u"  var element = document.getElementById('" + 
+               base::UTF8ToUTF16(id_it->second) + u"');";
+    js_code += u"  if (element) {";
+    js_code += u"    element.focus();";
+    js_code += u"    if (element.select) element.select();";  // Select text if possible
+    js_code += u"    return 'focused by id';";
+    js_code += u"  }";
+  }
+  
+  // Try to find by class and tag combination
+  auto class_it = node_info.attributes.find("class");
+  auto tag_it = node_info.attributes.find("html-tag");
+  
+  if (class_it != node_info.attributes.end() && !class_it->second.empty() &&
+      tag_it != node_info.attributes.end() && !tag_it->second.empty()) {
+    // Split class names and create selector
+    std::string class_selector = "." + class_it->second;
+    // Replace spaces with dots for multiple classes
+    for (size_t i = 0; i < class_selector.length(); ++i) {
+      if (class_selector[i] == ' ') {
+        class_selector[i] = '.';
+      }
+    }
+    
+    js_code += u"  var elements = document.querySelectorAll('" + 
+               base::UTF8ToUTF16(tag_it->second + class_selector) + u"');";
+    js_code += u"  if (elements.length > 0) {";
+    js_code += u"    elements[0].focus();";
+    js_code += u"    if (elements[0].select) elements[0].select();";
+    js_code += u"    return 'focused by class and tag';";
+    js_code += u"  }";
+  }
+  
+  // Fallback: try just by tag name if available
+  if (tag_it != node_info.attributes.end() && !tag_it->second.empty()) {
+    js_code += u"  var elements = document.getElementsByTagName('" + 
+               base::UTF8ToUTF16(tag_it->second) + u"');";
+    js_code += u"  if (elements.length > 0) {";
+    js_code += u"    elements[0].focus();";
+    js_code += u"    if (elements[0].select) elements[0].select();";
+    js_code += u"    return 'focused by tag';";
+    js_code += u"  }";
+  }
+  
+  js_code += u"  return 'no element found';";
+  js_code += u"})();";
+  
+  // Execute the JavaScript
+  rfh->ExecuteJavaScriptForTests(
+      js_code,
+      base::NullCallback(),
+      /*honor_js_content_settings=*/false);
+}
+
+// Helper to perform scroll actions using mouse wheel events
+void Scroll(content::WebContents* web_contents,
+                   int delta_x,
+                   int delta_y,
+                   bool precise) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh)
+    return;
+    
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv)
+    return;
+
+  // Get viewport bounds and center point
+  gfx::Rect viewport_bounds = rwhv->GetViewBounds();
+  gfx::PointF center_point(viewport_bounds.width() / 2.0f,
+                          viewport_bounds.height() / 2.0f);
+  
+  // Create mouse wheel event
+  blink::WebMouseWheelEvent wheel_event;
+  wheel_event.SetType(blink::WebInputEvent::Type::kMouseWheel);
+  wheel_event.SetPositionInWidget(center_point.x(), center_point.y());
+  wheel_event.SetPositionInScreen(center_point.x() + viewport_bounds.x(),
+                                 center_point.y() + viewport_bounds.y());
+  wheel_event.SetTimeStamp(ui::EventTimeForNow());
+  
+  // Set the scroll deltas
+  wheel_event.delta_x = delta_x;
+  wheel_event.delta_y = delta_y;
+  
+  // Set wheel tick values (120 = one notch)
+  wheel_event.wheel_ticks_x = delta_x / 120.0f;
+  wheel_event.wheel_ticks_y = delta_y / 120.0f;
+  
+  // Phase information for smooth scrolling
+  wheel_event.phase = blink::WebMouseWheelEvent::kPhaseBegan;
+  
+  // Precise scrolling for touchpad, non-precise for mouse wheel
+  if (precise) {
+    // For precise scrolling, deltas are in pixels
+    wheel_event.delta_units = ui::ScrollGranularity::kScrollByPrecisePixel;
+  } else {
+    // For non-precise scrolling, deltas are in lines
+    wheel_event.delta_units = ui::ScrollGranularity::kScrollByLine;
+  }
+  
+  // Send the wheel event
+  rwh->ForwardWheelEvent(wheel_event);
+  
+  // Send phase ended event for smooth scrolling
+  wheel_event.phase = blink::WebMouseWheelEvent::kPhaseEnded;
+  wheel_event.delta_x = 0;
+  wheel_event.delta_y = 0;
+  wheel_event.wheel_ticks_x = 0;
+  wheel_event.wheel_ticks_y = 0;
+  rwh->ForwardWheelEvent(wheel_event);
+}
+
+// Helper to send special key events
+void KeyPress(content::WebContents* web_contents,
+                    const std::string& key) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh)
+    return;
+
+  // Map key names to Windows key codes and DOM codes/keys
+  ui::KeyboardCode windows_key_code;
+  ui::DomCode dom_code;
+  ui::DomKey dom_key;
+  
+  // Use if-else chain to avoid static initialization
+  if (key == "Enter") {
+    windows_key_code = ui::VKEY_RETURN;
+    dom_code = ui::DomCode::ENTER;
+    dom_key = ui::DomKey::ENTER;
+  } else if (key == "Delete") {
+    windows_key_code = ui::VKEY_DELETE;
+    dom_code = ui::DomCode::DEL;
+    dom_key = ui::DomKey::DEL;
+  } else if (key == "Backspace") {
+    windows_key_code = ui::VKEY_BACK;
+    dom_code = ui::DomCode::BACKSPACE;
+    dom_key = ui::DomKey::BACKSPACE;
+  } else if (key == "Tab") {
+    windows_key_code = ui::VKEY_TAB;
+    dom_code = ui::DomCode::TAB;
+    dom_key = ui::DomKey::TAB;
+  } else if (key == "Escape") {
+    windows_key_code = ui::VKEY_ESCAPE;
+    dom_code = ui::DomCode::ESCAPE;
+    dom_key = ui::DomKey::ESCAPE;
+  } else if (key == "ArrowUp") {
+    windows_key_code = ui::VKEY_UP;
+    dom_code = ui::DomCode::ARROW_UP;
+    dom_key = ui::DomKey::ARROW_UP;
+  } else if (key == "ArrowDown") {
+    windows_key_code = ui::VKEY_DOWN;
+    dom_code = ui::DomCode::ARROW_DOWN;
+    dom_key = ui::DomKey::ARROW_DOWN;
+  } else if (key == "ArrowLeft") {
+    windows_key_code = ui::VKEY_LEFT;
+    dom_code = ui::DomCode::ARROW_LEFT;
+    dom_key = ui::DomKey::ARROW_LEFT;
+  } else if (key == "ArrowRight") {
+    windows_key_code = ui::VKEY_RIGHT;
+    dom_code = ui::DomCode::ARROW_RIGHT;
+    dom_key = ui::DomKey::ARROW_RIGHT;
+  } else if (key == "Home") {
+    windows_key_code = ui::VKEY_HOME;
+    dom_code = ui::DomCode::HOME;
+    dom_key = ui::DomKey::HOME;
+  } else if (key == "End") {
+    windows_key_code = ui::VKEY_END;
+    dom_code = ui::DomCode::END;
+    dom_key = ui::DomKey::END;
+  } else if (key == "PageUp") {
+    windows_key_code = ui::VKEY_PRIOR;
+    dom_code = ui::DomCode::PAGE_UP;
+    dom_key = ui::DomKey::PAGE_UP;
+  } else if (key == "PageDown") {
+    windows_key_code = ui::VKEY_NEXT;
+    dom_code = ui::DomCode::PAGE_DOWN;
+    dom_key = ui::DomKey::PAGE_DOWN;
+  } else {
+    return;  // Unsupported key
+  }
+  
+  // Create keyboard event
+  input::NativeWebKeyboardEvent key_down(
+      blink::WebInputEvent::Type::kKeyDown,
+      blink::WebInputEvent::kNoModifiers,
+      ui::EventTimeForNow());
+  
+  key_down.windows_key_code = windows_key_code;
+  key_down.native_key_code = windows_key_code;
+  key_down.dom_code = static_cast<int>(dom_code);
+  key_down.dom_key = static_cast<int>(dom_key);
+  
+  // Send key down
+  rwh->ForwardKeyboardEvent(key_down);
+  
+  // For Enter key, also send char event
+  // This is for `input` elements on web pages expect this to trigger submit
+  if (key == "Enter") {
+    input::NativeWebKeyboardEvent char_event(
+        blink::WebInputEvent::Type::kChar,
+        blink::WebInputEvent::kNoModifiers,
+        ui::EventTimeForNow());
+    
+    char_event.windows_key_code = windows_key_code;
+    char_event.native_key_code = windows_key_code;
+    char_event.dom_code = static_cast<int>(dom_code);
+    char_event.dom_key = static_cast<int>(dom_key);
+    char_event.text[0] = '\r';  // Carriage return character
+    char_event.unmodified_text[0] = '\r';
+    
+    rwh->ForwardKeyboardEvent(char_event);
+  }
+  
+  // For most keys, also send key up
+  if (key != "Tab") {  // Tab usually doesn't need key up for focus change
+    input::NativeWebKeyboardEvent key_up(
+        blink::WebInputEvent::Type::kKeyUp,
+        blink::WebInputEvent::kNoModifiers,
+        ui::EventTimeForNow());
+    
+    key_up.windows_key_code = windows_key_code;
+    key_up.native_key_code = windows_key_code;
+    key_up.dom_code = static_cast<int>(dom_code);
+    key_up.dom_key = static_cast<int>(dom_key);
+    
+    rwh->ForwardKeyboardEvent(key_up);
+  }
+}
+
+// Helper to type text into a focused element using native IME
+void NativeType(content::WebContents* web_contents,
+                const std::string& text) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+    
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh)
+    return;
+  
+  // Convert text to UTF16
+  std::u16string text16 = base::UTF8ToUTF16(text);
+  
+  // Immediately send the text without delay - focus should be handled before calling Type
+  content::RenderWidgetHostImpl* rwhi = 
+      static_cast<content::RenderWidgetHostImpl*>(rwh);
+  
+  // Ensure the widget has focus
+  rwhi->Focus();
+  
+  // Use ImeCommitText directly without composition for better compatibility
+  // This is more reliable for form inputs and avoids composition state issues
+  rwhi->ImeCommitText(text16,
+                      std::vector<ui::ImeTextSpan>(),
+                      gfx::Range::InvalidRange(),
+                      0);  // relative_cursor_pos = 0 means after the text
+}
+
+// Helper to set text value using JavaScript
+void JavaScriptType(content::WebContents* web_contents,
+                    const NodeInfo& node_info,
+                    const std::string& text) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh)
+    return;
+  
+  // Build JavaScript to find element and set its value
+  std::u16string js_code = u"(function() {";
+  std::u16string escaped_text = base::UTF8ToUTF16(text);
+  
+  // Escape quotes in the text
+  for (size_t i = 0; i < escaped_text.length(); ++i) {
+    if (escaped_text[i] == u'\'') {
+      escaped_text.insert(i, u"\\");
+      i++;
+    }
+  }
+  
+  // Try to find element by ID first
+  auto id_it = node_info.attributes.find("id");
+  if (id_it != node_info.attributes.end() && !id_it->second.empty()) {
+    js_code += u"  var element = document.getElementById('" + 
+               base::UTF8ToUTF16(id_it->second) + u"');";
+    js_code += u"  if (element) {";
+    js_code += u"    element.value = '" + escaped_text + u"';";
+    js_code += u"    element.dispatchEvent(new Event('input', {bubbles: true}));";
+    js_code += u"    element.dispatchEvent(new Event('change', {bubbles: true}));";
+    js_code += u"    return 'set by id';";
+    js_code += u"  }";
+  }
+  
+  // Try to find by class and tag combination
+  auto class_it = node_info.attributes.find("class");
+  auto tag_it = node_info.attributes.find("html-tag");
+  
+  if (class_it != node_info.attributes.end() && !class_it->second.empty() &&
+      tag_it != node_info.attributes.end() && !tag_it->second.empty()) {
+    std::string class_selector = "." + class_it->second;
+    for (size_t i = 0; i < class_selector.length(); ++i) {
+      if (class_selector[i] == ' ') {
+        class_selector[i] = '.';
+      }
+    }
+    
+    js_code += u"  var elements = document.querySelectorAll('" + 
+               base::UTF8ToUTF16(tag_it->second + class_selector) + u"');";
+    js_code += u"  if (elements.length > 0) {";
+    js_code += u"    if (elements[0].value !== undefined) {";
+    js_code += u"      elements[0].value = '" + escaped_text + u"';";
+    js_code += u"    } else if (elements[0].isContentEditable) {";
+    js_code += u"      elements[0].textContent = '" + escaped_text + u"';";
+    js_code += u"    }";
+    js_code += u"    elements[0].dispatchEvent(new Event('input', {bubbles: true}));";
+    js_code += u"    elements[0].dispatchEvent(new Event('change', {bubbles: true}));";
+    js_code += u"    return 'set by class and tag';";
+    js_code += u"  }";
+  }
+  
+  js_code += u"  return 'no element found';";
+  js_code += u"})();";
+  
+  // Execute the JavaScript
+  rfh->ExecuteJavaScriptForTests(
+      js_code,
+      base::NullCallback(),
+      /*honor_js_content_settings=*/false);
+}
+
+// Helper to perform accessibility action: DoDefault (click)
+bool AccessibilityDoDefault(content::WebContents* web_contents,
+                            const NodeInfo& node_info) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    LOG(WARNING) << "[browseros] No RenderFrameHost for AccessibilityDoDefault";
+    return false;
+  }
+  
+  ui::AXActionData action_data;
+  action_data.action = ax::mojom::Action::kDoDefault;
+  action_data.target_node_id = node_info.ax_node_id;
+  action_data.target_tree_id = node_info.ax_tree_id;
+  
+  LOG(INFO) << "[browseros] Performing AccessibilityDoDefault on node " 
+            << node_info.ax_node_id;
+  
+  rfh->AccessibilityPerformAction(action_data);
+  return true;
+}
+
+// Helper to perform accessibility action: Focus
+bool AccessibilityFocus(content::WebContents* web_contents,
+                       const NodeInfo& node_info) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    LOG(WARNING) << "[browseros] No RenderFrameHost for AccessibilityFocus";
+    return false;
+  }
+  
+  ui::AXActionData action_data;
+  action_data.action = ax::mojom::Action::kFocus;
+  action_data.target_node_id = node_info.ax_node_id;
+  action_data.target_tree_id = node_info.ax_tree_id;
+  
+  LOG(INFO) << "[browseros] Performing AccessibilityFocus on node " 
+            << node_info.ax_node_id;
+  
+  rfh->AccessibilityPerformAction(action_data);
+  return true;
+}
+
+// Helper to perform accessibility action: ScrollToMakeVisible
+bool AccessibilityScrollToMakeVisible(content::WebContents* web_contents,
+                                      const NodeInfo& node_info,
+                                      bool center_in_viewport) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    LOG(WARNING) << "[browseros] No RenderFrameHost for AccessibilityScrollToMakeVisible";
+    return false;
+  }
+  
+  ui::AXActionData action_data;
+  action_data.action = ax::mojom::Action::kScrollToMakeVisible;
+  action_data.target_node_id = node_info.ax_node_id;
+  action_data.target_tree_id = node_info.ax_tree_id;
+  
+  // Center the element in viewport for better visibility
+  if (center_in_viewport) {
+    action_data.horizontal_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentCenter;
+    action_data.vertical_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentCenter;
+  } else {
+    action_data.horizontal_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentClosestEdge;
+    action_data.vertical_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentClosestEdge;
+  }
+  
+  // Use kScrollIfVisible to only scroll if needed
+  action_data.scroll_behavior = ax::mojom::ScrollBehavior::kScrollIfVisible;
+  
+  LOG(INFO) << "[browseros] Performing AccessibilityScrollToMakeVisible on node " 
+            << node_info.ax_node_id;
+  
+  rfh->AccessibilityPerformAction(action_data);
+  return true;
+}
+
+// Helper to perform a click with change detection and retrying
+bool ClickWithDetection(content::WebContents* web_contents,
+                        const NodeInfo& node_info) {
+  // Check if node is out of viewport and needs scrolling
+  auto viewport_it = node_info.attributes.find("in_viewport");
+  bool is_out_of_viewport = (viewport_it != node_info.attributes.end() && 
+                              viewport_it->second == "false");
+  
+  if (is_out_of_viewport) {
+    LOG(INFO) << "[browseros] Node is out of viewport, scrolling to make visible";
+    AccessibilityScrollToMakeVisible(web_contents, node_info, true /* center */);
+    // Wait for scroll to complete
+    base::PlatformThread::Sleep(base::Milliseconds(300));
+    
+    // For out-of-viewport nodes, use AccessibilityDoDefault first (most reliable after scroll)
+    // LOG(INFO) << "[browseros] Node was out of viewport, trying AccessibilityDoDefault click first";
+    // bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+    //     web_contents,
+    //     [&]() { AccessibilityDoDefault(web_contents, node_info); },
+    //     base::Milliseconds(300));
+    
+    gfx::PointF click_point = GetNodeCenterPoint(web_contents, node_info);
+    
+    bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+        web_contents,
+        [&]() { PointClick(web_contents, click_point); },
+        base::Milliseconds(300));
+    
+    if (!changed) {
+      // Skip coordinate click for out-of-viewport nodes (coordinates unreliable)
+      // Go straight to HTML click
+      LOG(INFO) << "[browseros] No change from accessibility click, trying HTML click";
+      changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+          web_contents,
+          [&]() { HtmlClick(web_contents, node_info); },
+          base::Milliseconds(200));
+    }
+    
+    LOG(INFO) << "[browseros] Click result: " << (changed ? "changed" : "no change");
+    return changed;
+  }
+  
+  // For in-viewport nodes, try coordinate click first (most natural)
+  LOG(INFO) << "[browseros] Node is in viewport, trying coordinate click first";
+  gfx::PointF click_point = GetNodeCenterPoint(web_contents, node_info);
+  
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() { PointClick(web_contents, click_point); },
+      base::Milliseconds(300));
+  
+  // If still no change, try HTML click as final fallback
+  if (!changed) {
+      LOG(INFO) << "[browseros] No change from accessibility click, trying HTML click";
+      changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+          web_contents,
+          [&]() { HtmlClick(web_contents, node_info); },
+          base::Milliseconds(200));
+  }
+  
+  LOG(INFO) << "[browseros] Click result: " << (changed ? "changed" : "no change");
+  return changed;
+}
+
+// Helper to perform accessibility action: SetValue
+bool AccessibilitySetValue(content::WebContents* web_contents,
+                           const NodeInfo& node_info,
+                           const std::string& text) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    LOG(WARNING) << "[browseros] No RenderFrameHost for AccessibilitySetValue";
+    return false;
+  }
+  
+  ui::AXActionData action_data;
+  action_data.action = ax::mojom::Action::kSetValue;
+  action_data.target_node_id = node_info.ax_node_id;
+  action_data.target_tree_id = node_info.ax_tree_id;
+  action_data.value = text;
+  
+  LOG(INFO) << "[browseros] Performing AccessibilitySetValue on node " 
+            << node_info.ax_node_id << " with text: " << text;
+  
+  rfh->AccessibilityPerformAction(action_data);
+  return true;
+}
+
+// Helper to perform typing with change detection
+bool TypeWithDetection(content::WebContents* web_contents,
+                      const NodeInfo& node_info,
+                      const std::string& text) {
+  // Check if node is out of viewport and needs scrolling
+  auto viewport_it = node_info.attributes.find("in_viewport");
+  bool is_out_of_viewport = (viewport_it != node_info.attributes.end() && 
+                              viewport_it->second == "false");
+  
+  if (is_out_of_viewport) {
+    LOG(INFO) << "[browseros] Node is out of viewport for typing, scrolling to make visible";
+    AccessibilityScrollToMakeVisible(web_contents, node_info, true /* center */);
+    // Wait for scroll to complete
+    base::PlatformThread::Sleep(base::Milliseconds(300));
+  }
+  
+  // First ensure the element is focused using accessibility
+  LOG(INFO) << "[browseros] Focusing element for typing";
+  AccessibilityFocus(web_contents, node_info);
+  // Small delay to ensure focus is set
+  base::PlatformThread::Sleep(base::Milliseconds(50));
+  
+  // Try native typing first (most natural method)
+  LOG(INFO) << "[browseros] Trying native typing";
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() {
+        NativeType(web_contents, text);
+      },
+      base::Milliseconds(300));
+  
+  // If no change detected, try JavaScript typing as second fallback
+  if (!changed) {
+    LOG(INFO) << "[browseros] No change from native typing, trying JavaScript";
+    changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+        web_contents,
+        [&]() { JavaScriptType(web_contents, node_info, text); },
+        base::Milliseconds(200));
+  }
+  
+  // If still no change, try accessibility SetValue as final fallback
+  // if (!changed) {
+  //   LOG(INFO) << "[browseros] No change from JavaScript, trying accessibility SetValue";
+  //   changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+  //       web_contents,
+  //       [&]() {
+  //         AccessibilitySetValue(web_contents, node_info, text);
+  //       },
+  //       base::Milliseconds(300));
+  // }
+  
+  LOG(INFO) << "[browseros] Type result: " << (changed ? "changed" : "no change");
+  return changed;
+}
+
+// Helper to clear an input field with change detection
+bool ClearWithDetection(content::WebContents* web_contents,
+                       const NodeInfo& node_info) {
+  // Use change detection with JavaScript clear
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() {
+        content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+        if (!rfh) return;
+        
+        // First focus the element
+        HtmlFocus(web_contents, node_info);
+        
+        // Then clear using JavaScript
+        rfh->ExecuteJavaScriptForTests(
+            u"(function() {"
+            u"  var activeElement = document.activeElement;"
+            u"  if (activeElement) {"
+            u"    if (activeElement.value !== undefined) {"
+            u"      activeElement.value = '';"
+            u"    }"
+            u"    if (activeElement.textContent !== undefined && activeElement.isContentEditable) {"
+            u"      activeElement.textContent = '';"
+            u"    }"
+            u"    activeElement.dispatchEvent(new Event('input', {bubbles: true}));"
+            u"    activeElement.dispatchEvent(new Event('change', {bubbles: true}));"
+            u"  }"
+            u"})();",
+            base::NullCallback(),
+            /*honor_js_content_settings=*/false);
+      },
+      base::Milliseconds(200));
+  
+  LOG(INFO) << "[browseros] Clear result: " << (changed ? "changed" : "no change");
+  return changed;
+}
+
+// Helper to send a key press with change detection
+bool KeyPressWithDetection(content::WebContents* web_contents,
+                          const std::string& key) {
+  // Use change detection with key press
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() { KeyPress(web_contents, key); },
+      base::Milliseconds(200));
+  
+  LOG(INFO) << "[browseros] KeyPress result for '" << key << "': " 
+            << (changed ? "changed" : "no change");
+  return changed;
+}
+
+// Helper to show highlights for clickable, typeable, and selectable elements that are in viewport
+void ShowHighlights(
+    content::WebContents* web_contents,
+    const std::unordered_map<uint32_t, NodeInfo>& node_mappings,
+    bool show_labels) {
+  
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) return;
+  
+  // Filter nodes to only include:
+  // 1. Elements that are in viewport (using stored in_viewport field)
+  // 2. Elements that are clickable, typeable, or selectable (using stored node_type)
+  std::unordered_map<uint32_t, NodeInfo> filtered_nodes;
+  
+  for (const auto& [node_id, node_info] : node_mappings) {
+    // Check if element is in viewport using the stored field
+    if (!node_info.in_viewport) {
+      continue;  // Skip elements not in viewport
+    }
+    
+    // Check if element is clickable, typeable, or selectable using stored node_type
+    // Skip "other" interactive type as requested
+    if (node_info.node_type == blockbrowser::InteractiveNodeType::kClickable ||
+        node_info.node_type == blockbrowser::InteractiveNodeType::kTypeable ||
+        node_info.node_type == blockbrowser::InteractiveNodeType::kSelectable) {
+      filtered_nodes[node_id] = node_info;
+    }
+  }
+  
+  // If no nodes match our criteria, return early
+  if (filtered_nodes.empty()) {
+    LOG(INFO) << "[browseros] No interactive elements in viewport to highlight";
+    return;
+  }
+  
+  LOG(INFO) << "[browseros] Highlighting " << filtered_nodes.size() 
+            << " interactive elements in viewport (out of " << node_mappings.size() << " total)";
+  
+  // Use the original drawing implementation but with filtered nodes
+  
+  // Build JavaScript to draw all bounding boxes at once
+  std::string js_code = R"(
+    (function() {
+      // Remove any existing bounding boxes
+      document.querySelectorAll('.browseros-bbox').forEach(e => e.remove());
+      
+      // Create container for all bounding boxes
+      const container = document.createElement('div');
+      container.className = 'browseros-bbox-container';
+      container.style.cssText = `
+        position: fixed;
+        top: 0;
+        left: 0;
+        width: 100%;
+        height: 100%;
+        pointer-events: none;
+        z-index: 2147483647;
+      `;
+      
+      // Node data with bounds
+      const nodes = [
+  )";
+  
+  // Add filtered nodes with their bounds
+  bool first = true;
+  for (const auto& [node_id, node_info] : filtered_nodes) {
+    if (!first) js_code += ",";
+    first = false;
+    
+    // Bounds are already in CSS pixels from SnapshotProcessor
+    js_code += base::StringPrintf(
+        R"(
+        {
+          id: %d,
+          x: %f,
+          y: %f,
+          width: %f,
+          height: %f,
+          role: "%s"
+        })",
+        node_id,
+        node_info.bounds.x(),
+        node_info.bounds.y(),
+        node_info.bounds.width(),
+        node_info.bounds.height(),
+        node_info.attributes.count("role") ? node_info.attributes.at("role").c_str() : "unknown"
+    );
+  }
+  
+  js_code += R"(
+      ];
+      
+      // Draw bounding boxes for all nodes in parallel
+      nodes.forEach(node => {
+        if (node.width <= 0 || node.height <= 0) return;
+        
+        const box = document.createElement('div');
+        box.className = 'browseros-bbox';
+        box.dataset.nodeId = node.id;
+        box.style.cssText = `
+          position: absolute;
+          left: ${node.x}px;
+          top: ${node.y}px;
+          width: ${node.width}px;
+          height: ${node.height}px;
+          border: 2px solid #1E40AF;
+          background: transparent;
+          box-sizing: border-box;
+        `;
+  )";
+  
+  if (show_labels) {
+    js_code += R"(
+        // Add label with node ID
+        const label = document.createElement('div');
+        label.style.cssText = `
+          position: absolute;
+          top: -22px;
+          left: 0;
+          background: #2563EB;
+          color: #FFFFFF;
+          padding: 3px 7px;
+          font-size: 14px;
+          font-family: monospace;
+          border-radius: 3px;
+          white-space: nowrap;
+          opacity: 0.9;
+        `;
+        label.textContent = node.id;
+        box.appendChild(label);
+    )";
+  }
+  
+  js_code += R"(
+        container.appendChild(box);
+      });
+      
+      document.body.appendChild(container);
+      
+      // Return count for verification
+      return nodes.length;
+    })();
+  )";
+  
+  // Execute the JavaScript
+  rfh->ExecuteJavaScriptForTests(
+      base::UTF8ToUTF16(js_code),
+      base::NullCallback(),
+      false);
+}
+
+// Helper to remove all bounding box highlights from the page
+void RemoveHighlights(content::WebContents* web_contents) {
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) return;
+  
+  // JavaScript to remove all bounding box containers
+  std::string js_code = R"(
+    (function() {
+      // Remove all bounding box containers
+      document.querySelectorAll('.browseros-bbox-container').forEach(e => e.remove());
+      document.querySelectorAll('.browseros-bbox').forEach(e => e.remove());
+      
+      // Remove all highlight containers  
+      document.querySelectorAll('.browseros-highlight-container').forEach(e => e.remove());
+      document.querySelectorAll('.browseros-highlight').forEach(e => e.remove());
+      
+      // Remove any style elements we added
+      document.querySelectorAll('#browseros-highlight-styles').forEach(e => e.remove());
+      
+      return true;
+    })();
+  )";
+  
+  rfh->ExecuteJavaScriptForTests(
+      base::UTF8ToUTF16(js_code),
+      base::NullCallback(),
+      false);
+}
+
+// Helper to click at specific coordinates with change detection
+bool ClickCoordinatesWithDetection(content::WebContents* web_contents,
+                                   const gfx::PointF& point) {
+  LOG(INFO) << "[browseros] ClickCoordinatesWithDetection at (" 
+            << point.x() << ", " << point.y() << ")";
+  
+  // Perform coordinate click with change detection
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() { 
+        PointClick(web_contents, point);
+      },
+      base::Milliseconds(300));
+  
+  LOG(INFO) << "[browseros] Click coordinates result: " 
+            << (changed ? "changed" : "no change");
+  return changed;
+}
+
+// Helper to type text after clicking at coordinates to focus element
+bool TypeAtCoordinatesWithDetection(content::WebContents* web_contents,
+                                    const gfx::PointF& point,
+                                    const std::string& text) {
+  LOG(INFO) << "[browseros] TypeAtCoordinatesWithDetection at (" 
+            << point.x() << ", " << point.y() << ") with text: " << text;
+  
+  // First click at the coordinates to focus the element
+  PointClick(web_contents, point);
+  
+  // Wait a moment for focus to be established
+  base::PlatformThread::Sleep(base::Milliseconds(100));
+  
+  // Now type the text with change detection
+  bool changed = BlockBrowserChangeDetector::ExecuteWithDetection(
+      web_contents,
+      [&]() { 
+        NativeType(web_contents, text);
+      },
+      base::Milliseconds(300));
+  
+  // If native typing didn't work, try JavaScript injection to detect and type
+  if (!changed) {
+    LOG(INFO) << "[browseros] No change from native typing at coordinates, trying JS injection";
+    
+    // Execute JavaScript to find the focused element and set its value
+    content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+    if (rfh) {
+      std::string js_code = base::StringPrintf(R"(
+        (function() {
+          var focused = document.activeElement;
+          if (focused && (focused.tagName === 'INPUT' || 
+                         focused.tagName === 'TEXTAREA' || 
+                         focused.contentEditable === 'true')) {
+            if (focused.contentEditable === 'true') {
+              focused.textContent = '%s';
+            } else {
+              focused.value = '%s';
+            }
+            // Trigger input event
+            focused.dispatchEvent(new Event('input', { bubbles: true }));
+            focused.dispatchEvent(new Event('change', { bubbles: true }));
+            return true;
+          }
+          return false;
+        })();
+      )", text.c_str(), text.c_str());
+      
+      rfh->ExecuteJavaScriptForTests(
+          base::UTF8ToUTF16(js_code),
+          base::NullCallback(),
+          false);
+      
+      // Give it a moment to register
+      base::PlatformThread::Sleep(base::Milliseconds(50));
+      changed = true; // Assume success if we reached here
+    }
+  }
+  
+  LOG(INFO) << "[browseros] Type at coordinates result: " 
+            << (changed ? "success" : "failed");
+  return changed;
+}
+
+}  // namespace api
+}  // namespace extensions
