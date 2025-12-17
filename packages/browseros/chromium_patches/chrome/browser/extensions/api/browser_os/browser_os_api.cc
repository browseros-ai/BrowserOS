diff --git a/chrome/browser/extensions/api/browser_os/browser_os_api.cc b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
new file mode 100644
index 0000000000000..58abbe5b785ed
--- /dev/null
+++ b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
@@ -0,0 +1,1325 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/browser_os/browser_os_api.h"
+
+#include <set>
+#include <string>
+#include <unordered_map>
+#include <utility>
+#include <vector>
+
+#include "base/functional/bind.h"
+#include "base/threading/platform_thread.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/prefs/pref_service.h"
+#include "base/json/json_writer.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/base64.h"
+#include "base/time/time.h"
+#include "base/values.h"
+#include "base/version_info/version_info.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_api_helpers.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_api_utils.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_change_detector.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_content_processor.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_snapshot_processor.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/extensions/window_controller.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_finder.h"
+#include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "chrome/common/extensions/api/browser_os.h"
+#include "content/browser/renderer_host/render_widget_host_impl.h"
+#include "content/public/browser/render_frame_host.h"
+#include "content/public/browser/render_widget_host.h"
+#include "content/public/browser/render_widget_host_view.h"
+#include "content/browser/renderer_host/render_widget_host_view_base.h"
+#include "content/public/browser/web_contents.h"
+#include "third_party/blink/public/common/input/web_input_event.h"
+#include "third_party/blink/public/common/input/web_mouse_event.h"
+#include "ui/accessibility/ax_action_data.h"
+#include "ui/accessibility/ax_enum_util.h"
+#include "ui/accessibility/ax_mode.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_role_properties.h"
+#include "ui/accessibility/ax_tree_update.h"
+#include "ui/base/ime/ime_text_span.h"
+#include "ui/events/base_event_utils.h"
+#include "ui/events/keycodes/dom/dom_code.h"
+#include "ui/events/keycodes/dom/dom_key.h"
+#include "ui/events/keycodes/keyboard_codes.h"
+#include "ui/gfx/geometry/point_f.h"
+#include "ui/gfx/geometry/rect.h"
+#include "ui/gfx/geometry/rect_f.h"
+#include "ui/gfx/range/range.h"
+#include "ui/gfx/codec/png_codec.h"
+#include "ui/gfx/image/image.h"
+#include "ui/snapshot/snapshot.h"
+
+namespace extensions {
+namespace api {
+
+namespace {
+
+// Serializes ui::AXNodeData to base::Value::Dict with all fields
+base::Value::Dict SerializeAXNodeData(const ui::AXNodeData& node) {
+  base::Value::Dict dict;
+
+  // Core identity
+  dict.Set("id", node.id);
+  dict.Set("role", ui::ToString(node.role));
+
+  // Hierarchy
+  if (!node.child_ids.empty()) {
+    base::Value::List children;
+    for (int32_t child_id : node.child_ids) {
+      children.Append(child_id);
+    }
+    dict.Set("childIds", std::move(children));
+  }
+
+  // State bitfield converted to string array
+  base::Value::List states;
+  for (int i = static_cast<int>(ax::mojom::State::kMinValue);
+       i <= static_cast<int>(ax::mojom::State::kMaxValue); ++i) {
+    auto state = static_cast<ax::mojom::State>(i);
+    if (node.HasState(state)) {
+      states.Append(ui::ToString(state));
+    }
+  }
+  if (!states.empty()) {
+    dict.Set("states", std::move(states));
+  }
+
+  // Actions bitfield converted to string array
+  base::Value::List actions;
+  for (int i = static_cast<int>(ax::mojom::Action::kMinValue);
+       i <= static_cast<int>(ax::mojom::Action::kMaxValue); ++i) {
+    auto action = static_cast<ax::mojom::Action>(i);
+    if (node.HasAction(action)) {
+      actions.Append(ui::ToString(action));
+    }
+  }
+  if (!actions.empty()) {
+    dict.Set("actions", std::move(actions));
+  }
+
+  // String attributes map with enum keys converted to strings
+  if (node.string_attributes.size() > 0) {
+    base::Value::Dict attrs;
+    for (const auto& [key, value] : node.string_attributes) {
+      attrs.Set(ui::ToString(key), value);
+    }
+    dict.Set("stringAttributes", std::move(attrs));
+  }
+
+  // Int attributes map
+  if (node.int_attributes.size() > 0) {
+    base::Value::Dict attrs;
+    for (const auto& [key, value] : node.int_attributes) {
+      attrs.Set(ui::ToString(key), value);
+    }
+    dict.Set("intAttributes", std::move(attrs));
+  }
+
+  // Float attributes map
+  if (node.float_attributes.size() > 0) {
+    base::Value::Dict attrs;
+    for (const auto& [key, value] : node.float_attributes) {
+      attrs.Set(ui::ToString(key), static_cast<double>(value));
+    }
+    dict.Set("floatAttributes", std::move(attrs));
+  }
+
+  // Bool attributes map
+  if (node.bool_attributes && node.bool_attributes->Size() > 0) {
+    base::Value::Dict attrs;
+    node.bool_attributes->ForEach([&attrs](ax::mojom::BoolAttribute key, bool value) {
+      attrs.Set(ui::ToString(key), value);
+    });
+    dict.Set("boolAttributes", std::move(attrs));
+  }
+
+  // IntList attributes map
+  if (node.intlist_attributes.size() > 0) {
+    base::Value::Dict attrs;
+    for (const auto& [key, values] : node.intlist_attributes) {
+      base::Value::List list;
+      for (int v : values) {
+        list.Append(v);
+      }
+      attrs.Set(ui::ToString(key), std::move(list));
+    }
+    dict.Set("intListAttributes", std::move(attrs));
+  }
+
+  // StringList attributes map
+  if (node.stringlist_attributes.size() > 0) {
+    base::Value::Dict attrs;
+    for (const auto& [key, values] : node.stringlist_attributes) {
+      base::Value::List list;
+      for (const auto& v : values) {
+        list.Append(v);
+      }
+      attrs.Set(ui::ToString(key), std::move(list));
+    }
+    dict.Set("stringListAttributes", std::move(attrs));
+  }
+
+  // HTML attributes (name-value pairs)
+  if (!node.html_attributes.empty()) {
+    base::Value::Dict attrs;
+    for (const auto& [name, value] : node.html_attributes) {
+      attrs.Set(name, value);
+    }
+    dict.Set("htmlAttributes", std::move(attrs));
+  }
+
+  return dict;
+}
+
+// Serializes ui::AXTreeData to base::Value::Dict
+base::Value::Dict SerializeAXTreeData(const ui::AXTreeData& tree_data) {
+  base::Value::Dict dict;
+
+  // Document metadata
+  if (!tree_data.title.empty()) {
+    dict.Set("title", tree_data.title);
+  }
+  if (!tree_data.url.empty()) {
+    dict.Set("url", tree_data.url);
+  }
+  if (!tree_data.doctype.empty()) {
+    dict.Set("doctype", tree_data.doctype);
+  }
+  if (!tree_data.mimetype.empty()) {
+    dict.Set("mimetype", tree_data.mimetype);
+  }
+
+  // Loading state
+  dict.Set("loaded", tree_data.loaded);
+  dict.Set("loadingProgress", tree_data.loading_progress);
+
+  // Focus
+  if (tree_data.focus_id != -1) {
+    dict.Set("focusId", tree_data.focus_id);
+  }
+
+  // Selection
+  if (tree_data.sel_anchor_object_id != -1) {
+    base::Value::Dict selection;
+    selection.Set("anchorObjectId", tree_data.sel_anchor_object_id);
+    selection.Set("anchorOffset", tree_data.sel_anchor_offset);
+    selection.Set("focusObjectId", tree_data.sel_focus_object_id);
+    selection.Set("focusOffset", tree_data.sel_focus_offset);
+    selection.Set("isBackward", tree_data.sel_is_backward);
+    dict.Set("selection", std::move(selection));
+  }
+
+  return dict;
+}
+
+// Helper to find which PrefService contains a preference
+// Tries Local State first, then Profile prefs
+PrefService* FindPrefService(const std::string& pref_name, Profile* profile) {
+  PrefService* local_state = g_browser_process->local_state();
+  if (local_state && local_state->FindPreference(pref_name)) {
+    return local_state;
+  }
+
+  PrefService* profile_prefs = profile->GetPrefs();
+  if (profile_prefs && profile_prefs->FindPreference(pref_name)) {
+    return profile_prefs;
+  }
+
+  return nullptr;
+}
+
+// Helper to determine preference type name from value
+std::string GetPrefTypeName(const base::Value* value) {
+  switch (value->type()) {
+    case base::Value::Type::BOOLEAN:
+      return "boolean";
+    case base::Value::Type::INTEGER:
+    case base::Value::Type::DOUBLE:
+      return "number";
+    case base::Value::Type::STRING:
+      return "string";
+    case base::Value::Type::LIST:
+      return "list";
+    case base::Value::Type::DICT:
+      return "dictionary";
+    default:
+      return "unknown";
+  }
+}
+
+}  // namespace
+
+// Static member initialization
+uint32_t BrowserOSGetInteractiveSnapshotFunction::next_snapshot_id_ = 1;
+
+// Constructor and destructor implementations
+BrowserOSGetInteractiveSnapshotFunction::BrowserOSGetInteractiveSnapshotFunction() = default;
+BrowserOSGetInteractiveSnapshotFunction::~BrowserOSGetInteractiveSnapshotFunction() = default;
+
+ExtensionFunction::ResponseAction BrowserOSGetAccessibilityTreeFunction::Run() {
+  std::optional<browser_os::GetAccessibilityTree::Params> params =
+      browser_os::GetAccessibilityTree::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+
+  // Enable accessibility if needed
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+
+  // Request accessibility tree snapshot
+  // Use WebContents with extended properties to get a full tree
+  web_contents->RequestAXTreeSnapshot(
+      base::BindOnce(
+          &BrowserOSGetAccessibilityTreeFunction::OnAccessibilityTreeReceived,
+          this),
+      ui::AXMode(ui::AXMode::kWebContents | ui::AXMode::kExtendedProperties |
+                 ui::AXMode::kInlineTextBoxes),
+      /* max_nodes= */ 0,  // No limit
+      /* timeout= */ base::TimeDelta(),
+      content::WebContents::AXTreeSnapshotPolicy::kAll);
+
+  return RespondLater();
+}
+
+void BrowserOSGetAccessibilityTreeFunction::OnAccessibilityTreeReceived(
+    ui::AXTreeUpdate& tree_update) {
+  browser_os::AccessibilityTree result;
+  result.root_id = tree_update.root_id;
+
+  // Serialize all nodes with complete AX data
+  base::Value::Dict nodes;
+  for (const auto& node_data : tree_update.nodes) {
+    nodes.Set(base::NumberToString(node_data.id),
+              SerializeAXNodeData(node_data));
+  }
+  result.nodes.additional_properties = std::move(nodes);
+
+  // Serialize tree-level metadata
+  browser_os::AccessibilityTree::TreeData tree_data_obj;
+  tree_data_obj.additional_properties = SerializeAXTreeData(tree_update.tree_data);
+  result.tree_data = std::move(tree_data_obj);
+
+  Respond(ArgumentList(
+      browser_os::GetAccessibilityTree::Results::Create(result)));
+}
+
+// Implementation of BrowserOSGetInteractiveSnapshotFunction
+
+ExtensionFunction::ResponseAction BrowserOSGetInteractiveSnapshotFunction::Run() {
+  std::optional<browser_os::GetInteractiveSnapshot::Params> params =
+      browser_os::GetInteractiveSnapshot::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  web_contents_ = web_contents;  // Store for later use in OnSnapshotProcessed
+  
+  // Note: We don't need to get scale factors here!
+  // The accessibility tree provides bounds in CSS pixels (logical pixels),
+  // which is the correct coordinate space for ForwardMouseEvent.
+  // The browser and renderer handle device pixel ratio conversion internally.
+  
+  // Store tab ID for mapping
+  tab_id_ = tab_info->tab_id;
+
+  // Check frame stability before requesting snapshot
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh || !rfh->IsRenderFrameLive() || !rfh->IsActive()) {
+    LOG(WARNING) << "[browseros] Frame not stable for AX snapshot - skipping";
+    browser_os::InteractiveSnapshot empty_snapshot;
+    empty_snapshot.snapshot_id = next_snapshot_id_++;
+    empty_snapshot.timestamp = base::Time::Now().InMillisecondsFSinceUnixEpoch();
+    empty_snapshot.processing_time_ms = 0;
+    return RespondNow(ArgumentList(
+        browser_os::GetInteractiveSnapshot::Results::Create(empty_snapshot)));
+  }
+  
+  // Request accessibility tree snapshot
+  web_contents->RequestAXTreeSnapshot(
+      base::BindOnce(
+          &BrowserOSGetInteractiveSnapshotFunction::OnAccessibilityTreeReceived,
+          this),
+      ui::AXMode(ui::AXMode::kWebContents | ui::AXMode::kExtendedProperties |
+                 ui::AXMode::kInlineTextBoxes),
+      /* max_nodes= */ 0,  // No limit
+      /* timeout= */ base::TimeDelta(),
+      content::WebContents::AXTreeSnapshotPolicy::kAll);
+      // content::WebContents::AXTreeSnapshotPolicy::kSameOriginDirectDescendants);
+
+  return RespondLater();
+}
+
+void BrowserOSGetInteractiveSnapshotFunction::OnAccessibilityTreeReceived(
+    ui::AXTreeUpdate& tree_update) {
+  // Double-check frame is still valid before processing
+  if (!web_contents_) {
+    LOG(WARNING) << "[browseros] WebContents gone during AX snapshot callback";
+    browser_os::InteractiveSnapshot empty_snapshot;
+    empty_snapshot.snapshot_id = next_snapshot_id_++;
+    empty_snapshot.timestamp = base::Time::Now().InMillisecondsFSinceUnixEpoch();
+    empty_snapshot.processing_time_ms = 0;
+    Respond(ArgumentList(
+        browser_os::GetInteractiveSnapshot::Results::Create(empty_snapshot)));
+    return;
+  }
+  
+  content::RenderFrameHost* rfh = web_contents_->GetPrimaryMainFrame();
+  if (!rfh || !rfh->IsRenderFrameLive()) {
+    LOG(WARNING) << "[browseros] Frame became unstable during AX snapshot callback";
+    browser_os::InteractiveSnapshot empty_snapshot;
+    empty_snapshot.snapshot_id = next_snapshot_id_++;
+    empty_snapshot.timestamp = base::Time::Now().InMillisecondsFSinceUnixEpoch();
+    empty_snapshot.processing_time_ms = 0;
+    Respond(ArgumentList(
+        browser_os::GetInteractiveSnapshot::Results::Create(empty_snapshot)));
+    return;
+  }
+  
+  // Simple API layer - just delegates to the processor
+  SnapshotProcessor::ProcessAccessibilityTree(
+      tree_update,
+      tab_id_,
+      next_snapshot_id_++,
+      web_contents_,
+      base::BindOnce(
+          &BrowserOSGetInteractiveSnapshotFunction::OnSnapshotProcessed,
+          base::WrapRefCounted(this)));
+}
+
+void BrowserOSGetInteractiveSnapshotFunction::OnSnapshotProcessed(
+    SnapshotProcessingResult result) {
+  Respond(ArgumentList(
+      browser_os::GetInteractiveSnapshot::Results::Create(result.snapshot)));
+}
+
+// Implementation of BrowserOSClickFunction
+
+ExtensionFunction::ResponseAction BrowserOSClickFunction::Run() {
+  std::optional<browser_os::Click::Params> params =
+      browser_os::Click::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  int tab_id = tab_info->tab_id;
+
+  // Look up the AX node ID from our nodeId
+  auto tab_it = GetNodeIdMappings().find(tab_id);
+  if (tab_it == GetNodeIdMappings().end()) {
+    return RespondNow(Error("No snapshot data for this tab"));
+  }
+  
+  auto node_it = tab_it->second.find(params->node_id);
+  if (node_it == tab_it->second.end()) {
+    return RespondNow(Error("Node ID not found"));
+  }
+  
+  const NodeInfo& node_info = node_it->second;
+  
+  // Perform click with change detection
+  bool change_detected = ClickWithDetection(web_contents, node_info);
+  
+  // Create interaction response
+  browser_os::InteractionResponse response;
+  response.success = change_detected;
+  
+  return RespondNow(ArgumentList(
+      browser_os::Click::Results::Create(response)));
+}
+
+// Implementation of BrowserOSInputTextFunction
+
+ExtensionFunction::ResponseAction BrowserOSInputTextFunction::Run() {
+  std::optional<browser_os::InputText::Params> params =
+      browser_os::InputText::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  int tab_id = tab_info->tab_id;
+
+  // Look up the AX node ID from our nodeId
+  auto tab_it = GetNodeIdMappings().find(tab_id);
+  if (tab_it == GetNodeIdMappings().end()) {
+    return RespondNow(Error("No snapshot data for this tab"));
+  }
+  
+  auto node_it = tab_it->second.find(params->node_id);
+  if (node_it == tab_it->second.end()) {
+    return RespondNow(Error("Node ID not found"));
+  }
+  
+  const NodeInfo& node_info = node_it->second;
+  
+  LOG(INFO) << "[browseros] InputText: Starting input for nodeId: " << params->node_id;
+  
+  // Use TypeWithDetection which tries both native and JavaScript methods
+  bool change_detected = TypeWithDetection(web_contents, node_info, params->text);
+  
+  if (!change_detected) {
+    LOG(WARNING) << "[browseros] InputText: No change detected after typing";
+  }
+  
+  // Create interaction response
+  browser_os::InteractionResponse response;
+  response.success = change_detected;
+  
+  return RespondNow(ArgumentList(
+      browser_os::InputText::Results::Create(response)));
+}
+
+// Implementation of BrowserOSClearFunction
+
+ExtensionFunction::ResponseAction BrowserOSClearFunction::Run() {
+  std::optional<browser_os::Clear::Params> params =
+      browser_os::Clear::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  int tab_id = tab_info->tab_id;
+
+  // Look up the AX node ID from our nodeId
+  auto tab_it = GetNodeIdMappings().find(tab_id);
+  if (tab_it == GetNodeIdMappings().end()) {
+    return RespondNow(Error("No snapshot data for this tab"));
+  }
+  
+  auto node_it = tab_it->second.find(params->node_id);
+  if (node_it == tab_it->second.end()) {
+    return RespondNow(Error("Node ID not found"));
+  }
+  
+  const NodeInfo& node_info = node_it->second;
+  
+  LOG(INFO) << "[browseros] Clear: Clearing field for nodeId: " << params->node_id;
+  
+  // Use ClearWithDetection which handles focus and clearing
+  bool change_detected = ClearWithDetection(web_contents, node_info);
+  
+  if (!change_detected) {
+    LOG(WARNING) << "[browseros] Clear: No change detected after clearing";
+  }
+  
+  // Create interaction response
+  browser_os::InteractionResponse response;
+  response.success = change_detected;
+  
+  return RespondNow(ArgumentList(
+      browser_os::Clear::Results::Create(response)));
+}
+
+// Implementation of BrowserOSGetPageLoadStatusFunction
+
+ExtensionFunction::ResponseAction BrowserOSGetPageLoadStatusFunction::Run() {
+  std::optional<browser_os::GetPageLoadStatus::Params> params =
+      browser_os::GetPageLoadStatus::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Get the primary main frame
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  // Build the status object
+  browser_os::PageLoadStatus status;
+  
+  // Check if any resources are still loading
+  status.is_resources_loading = web_contents->IsLoading();
+  
+  // Check if DOMContentLoaded has fired
+  status.is_dom_content_loaded = rfh->IsDOMContentLoaded();
+  
+  // Check if onload has completed (all resources loaded)
+  status.is_page_complete = rfh->IsDocumentOnLoadCompletedInMainFrame();
+  
+  return RespondNow(ArgumentList(
+      browser_os::GetPageLoadStatus::Results::Create(status)));
+}
+
+// Implementation of BrowserOSScrollUpFunction
+
+ExtensionFunction::ResponseAction BrowserOSScrollUpFunction::Run() {
+  std::optional<browser_os::ScrollUp::Params> params =
+      browser_os::ScrollUp::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Get viewport height to scroll by approximately one page
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh) {
+    return RespondNow(Error("No render widget host"));
+  }
+  
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv) {
+    return RespondNow(Error("No render widget host view"));
+  }
+  
+  gfx::Rect viewport_bounds = rwhv->GetViewBounds();
+  int scroll_amount = viewport_bounds.height() * 0.9;  // 90% of viewport height
+  
+  // Perform scroll up (negative delta_y)
+  Scroll(web_contents, 0, -scroll_amount, true);
+  
+  return RespondNow(NoArguments());
+}
+
+// Implementation of BrowserOSScrollDownFunction
+
+ExtensionFunction::ResponseAction BrowserOSScrollDownFunction::Run() {
+  std::optional<browser_os::ScrollDown::Params> params =
+      browser_os::ScrollDown::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Get viewport height to scroll by approximately one page
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh) {
+    return RespondNow(Error("No render widget host"));
+  }
+  
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv) {
+    return RespondNow(Error("No render widget host view"));
+  }
+  
+  gfx::Rect viewport_bounds = rwhv->GetViewBounds();
+  int scroll_amount = viewport_bounds.height() * 0.9;  // 90% of viewport height
+  
+  // Perform scroll down (positive delta_y)
+  Scroll(web_contents, 0, scroll_amount, true);
+  
+  return RespondNow(NoArguments());
+}
+
+// Implementation of BrowserOSScrollToNodeFunction
+
+ExtensionFunction::ResponseAction BrowserOSScrollToNodeFunction::Run() {
+  std::optional<browser_os::ScrollToNode::Params> params =
+      browser_os::ScrollToNode::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  int tab_id = tab_info->tab_id;
+  
+  // Look up the AX node ID from our nodeId
+  auto tab_it = GetNodeIdMappings().find(tab_id);
+  if (tab_it == GetNodeIdMappings().end()) {
+    return RespondNow(Error("No snapshot data for this tab"));
+  }
+  
+  auto node_it = tab_it->second.find(params->node_id);
+  if (node_it == tab_it->second.end()) {
+    return RespondNow(Error("Node ID not found"));
+  }
+  
+  const NodeInfo& node_info = node_it->second;
+  
+  // Get viewport bounds to check if node is already in view
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh) {
+    return RespondNow(Error("No render widget host"));
+  }
+  
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv) {
+    return RespondNow(Error("No render widget host view"));
+  }
+  
+  gfx::Rect viewport_bounds = rwhv->GetViewBounds();
+  
+  // Check if the node is already visible in the viewport
+  // We consider it visible if any part of it is within the viewport
+  bool is_in_view = false;
+  if (node_info.bounds.y() < viewport_bounds.height() && 
+      node_info.bounds.bottom() > 0 &&
+      node_info.bounds.x() < viewport_bounds.width() &&
+      node_info.bounds.right() > 0) {
+    is_in_view = true;
+  }
+  
+  if (!is_in_view) {
+    // Use accessibility action to scroll
+    if (rfh) {
+      ui::AXActionData action_data;
+      action_data.action = ax::mojom::Action::kScrollToMakeVisible;
+      action_data.target_node_id = node_info.ax_node_id;
+      action_data.horizontal_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentCenter;
+      action_data.vertical_scroll_alignment = ax::mojom::ScrollAlignment::kScrollAlignmentCenter;
+      action_data.scroll_behavior = ax::mojom::ScrollBehavior::kScrollIfVisible;
+      
+      rfh->AccessibilityPerformAction(action_data);
+    }
+  }
+  
+  return RespondNow(ArgumentList(
+      browser_os::ScrollToNode::Results::Create(!is_in_view)));
+}
+
+// Implementation of BrowserOSSendKeysFunction
+
+ExtensionFunction::ResponseAction BrowserOSSendKeysFunction::Run() {
+  std::optional<browser_os::SendKeys::Params> params =
+      browser_os::SendKeys::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Validate the key - use a simple check instead of std::set to avoid exit-time destructor
+  const std::string& key = params->key;
+  bool is_supported = (key == "Enter" || key == "Delete" || key == "Backspace" ||
+                       key == "Tab" || key == "Escape" || key == "ArrowUp" ||
+                       key == "ArrowDown" || key == "ArrowLeft" || key == "ArrowRight" ||
+                       key == "Home" || key == "End" || key == "PageUp" || key == "PageDown");
+  
+  if (!is_supported) {
+    return RespondNow(Error("Unsupported key: " + params->key));
+  }
+  
+  LOG(INFO) << "[browseros] SendKeys: Sending key '" << params->key << "'";
+  
+  // Send the key with change detection
+  bool change_detected = KeyPressWithDetection(web_contents, params->key);
+  
+  if (!change_detected) {
+    LOG(WARNING) << "[browseros] SendKeys: No change detected after key press";
+  }
+  
+  // Create interaction response
+  browser_os::InteractionResponse response;
+  response.success = change_detected;
+  
+  return RespondNow(ArgumentList(
+      browser_os::SendKeys::Results::Create(response)));
+}
+
+// Implementation of BrowserOSCaptureScreenshotFunction
+
+BrowserOSCaptureScreenshotFunction::BrowserOSCaptureScreenshotFunction() = default;
+BrowserOSCaptureScreenshotFunction::~BrowserOSCaptureScreenshotFunction() = default;
+
+ExtensionFunction::ResponseAction BrowserOSCaptureScreenshotFunction::Run() {
+  std::optional<browser_os::CaptureScreenshot::Params> params =
+      browser_os::CaptureScreenshot::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+  
+  // Store whether to show highlights
+  show_highlights_ = params->show_highlights.value_or(false);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  web_contents_ = web_contents;
+  tab_id_ = tab_info->tab_id;
+  
+  // Get the render widget host view
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh) {
+    return RespondNow(Error("No render widget host"));
+  }
+  
+  content::RenderWidgetHostView* rwhv = rwh->GetView();
+  if (!rwhv) {
+    return RespondNow(Error("No render widget host view"));
+  }
+  
+  // Get the view bounds to determine the size
+  gfx::Rect view_bounds = rwhv->GetViewBounds();
+  
+  // Check if exact width and height are specified
+  if (params->width && params->height) {
+    // Use exact dimensions without preserving aspect ratio
+    use_exact_dimensions_ = true;
+    target_size_ = gfx::Size(static_cast<int>(*params->width), 
+                            static_cast<int>(*params->height));
+    LOG(INFO) << "[browseros] CaptureScreenshot: Using exact dimensions: "
+              << target_size_.width() << "x" << target_size_.height();
+  } else {
+    // Fall back to original behavior with thumbnailSize
+    use_exact_dimensions_ = false;
+    
+    // Determine max thumbnail size
+    // If thumbnailSize is provided, use minimum of it and viewport dimensions
+    // Otherwise, use viewport size (no scaling)
+    int max_dimension;
+    if (params->thumbnail_size) {
+      // Take minimum of requested size and viewport dimensions
+      int viewport_max = std::max(view_bounds.width(), view_bounds.height());
+      max_dimension = std::min(static_cast<int>(*params->thumbnail_size), viewport_max);
+      LOG(INFO) << "[browseros] CaptureScreenshot: Using thumbnail size: " << max_dimension 
+                << " (requested: " << *params->thumbnail_size 
+                << ", viewport max: " << viewport_max << ")";
+    } else {
+      // No thumbnail size specified, use viewport dimensions
+      max_dimension = std::max(view_bounds.width(), view_bounds.height());
+      LOG(INFO) << "[browseros] CaptureScreenshot: Using viewport size: " << max_dimension;
+    }
+    
+    gfx::Size thumbnail_size = view_bounds.size();
+    
+    // Scale down proportionally if needed
+    if (thumbnail_size.width() > max_dimension || 
+        thumbnail_size.height() > max_dimension) {
+      float scale = std::min(
+          static_cast<float>(max_dimension) / thumbnail_size.width(),
+          static_cast<float>(max_dimension) / thumbnail_size.height());
+      thumbnail_size = gfx::ScaleToFlooredSize(thumbnail_size, scale);
+    }
+    
+    target_size_ = thumbnail_size;
+  }
+  
+  // Store target size for later use
+  
+  // Draw highlights first, then capture after a short delay
+  DrawHighlightsAndCapture();
+  
+  return RespondLater();
+}
+
+void BrowserOSCaptureScreenshotFunction::DrawHighlightsAndCapture() {
+  // Only draw highlights if requested via the showHighlights flag
+  if (show_highlights_) {
+    // Check if we have snapshot data for this tab to draw highlights
+    auto tab_it = GetNodeIdMappings().find(tab_id_);
+    if (tab_it != GetNodeIdMappings().end() && !tab_it->second.empty()) {
+      LOG(INFO) << "[browseros] Drawing highlights for screenshot with " 
+                << tab_it->second.size() << " interactive elements";
+      ShowHighlights(web_contents_, tab_it->second, true /* show_labels */);
+    } else {
+      LOG(INFO) << "[browseros] No snapshot data available for highlighting";
+    }
+    
+    // Use PostDelayedTask to allow the renderer to paint the highlights
+    // This lets the event loop run and process the DOM changes
+    // Use scoped_refptr to keep the function alive
+    base::SequencedTaskRunner::GetCurrentDefault()->PostDelayedTask(
+        FROM_HERE,
+        base::BindOnce(&BrowserOSCaptureScreenshotFunction::CaptureScreenshotNow,
+                       base::WrapRefCounted(this)),
+        base::Milliseconds(1000));  // Give enough time for JS execution and paint
+  } else {
+    // No highlights needed, capture immediately
+    CaptureScreenshotNow();
+  }
+}
+
+void BrowserOSCaptureScreenshotFunction::CaptureScreenshotNow() {
+  if (!web_contents_) {
+    Respond(Error("Web contents destroyed"));
+    return;
+  }
+  
+  content::RenderFrameHost* rfh = web_contents_->GetPrimaryMainFrame();
+  if (!rfh) {
+    Respond(Error("No render frame"));
+    return;
+  }
+  
+  content::RenderWidgetHost* rwh = rfh->GetRenderWidgetHost();
+  if (!rwh) {
+    Respond(Error("No render widget host"));
+    return;
+  }
+  
+  content::RenderWidgetHostImpl* rwhi = 
+      static_cast<content::RenderWidgetHostImpl*>(rwh);
+  
+  // Request the screenshot
+  rwhi->GetView()->CopyFromSurface(
+      gfx::Rect(),  // Empty rect means copy entire surface
+      target_size_,
+      base::BindOnce(&BrowserOSCaptureScreenshotFunction::OnScreenshotCaptured,
+                     this));
+}
+
+void BrowserOSCaptureScreenshotFunction::OnScreenshotCaptured(
+    const SkBitmap& bitmap) {
+  // Clean up the highlights immediately after capture (only if we added them)
+  if (show_highlights_ && web_contents_) {
+    RemoveHighlights(web_contents_);
+  }
+  
+  if (bitmap.empty()) {
+    Respond(Error("Failed to capture screenshot"));
+    return;
+  }
+  
+  // Convert bitmap to PNG
+  auto png_data = gfx::PNGCodec::EncodeBGRASkBitmap(bitmap, false);
+  if (!png_data.has_value()) {
+    Respond(Error("Failed to encode screenshot"));
+    return;
+  }
+  
+  // Convert to base64 data URL
+  std::string base64_data = base::Base64Encode(png_data.value());
+  
+  std::string data_url = "data:image/png;base64," + base64_data;
+  
+  Respond(ArgumentList(
+      browser_os::CaptureScreenshot::Results::Create(data_url)));
+}
+
+// BrowserOSGetSnapshotFunction implementation
+ExtensionFunction::ResponseAction BrowserOSGetSnapshotFunction::Run() {
+  auto params = browser_os::GetSnapshot::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+  
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Request accessibility tree snapshot
+  web_contents->RequestAXTreeSnapshot(
+      base::BindOnce(&BrowserOSGetSnapshotFunction::OnAccessibilityTreeReceived,
+                     this),
+      ui::AXMode(ui::AXMode::kWebContents | ui::AXMode::kExtendedProperties),
+      /* max_nodes= */ 0,  // No limit
+      /* timeout= */ base::TimeDelta(),
+      content::WebContents::AXTreeSnapshotPolicy::kAll);
+  
+  return RespondLater();
+}
+
+void BrowserOSGetSnapshotFunction::OnAccessibilityTreeReceived(
+    ui::AXTreeUpdate& tree_update) {
+  if (!has_callback()) {
+    return;
+  }
+
+  // Extract page content using the processor
+  base::Time start_time = base::Time::Now();
+  auto items = ContentProcessor::ExtractPageContent(tree_update);
+
+  // Build result
+  browser_os::PageContent result;
+  result.items = std::move(items);
+  result.timestamp = base::Time::Now().InMillisecondsFSinceUnixEpoch();
+  result.processing_time_ms =
+      (base::Time::Now() - start_time).InMilliseconds();
+
+  Respond(ArgumentList(browser_os::GetSnapshot::Results::Create(result)));
+}
+
+// BrowserOSGetPrefFunction
+ExtensionFunction::ResponseAction BrowserOSGetPrefFunction::Run() {
+  std::optional<browser_os::GetPref::Params> params =
+      browser_os::GetPref::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  PrefService* prefs = FindPrefService(params->name, profile);
+
+  if (!prefs) {
+    return RespondNow(Error("Preference not found: " + params->name));
+  }
+
+  browser_os::PrefObject pref_obj;
+  pref_obj.key = params->name;
+
+  const base::Value* value = prefs->GetUserPrefValue(params->name);
+  if (!value) {
+    value = prefs->GetDefaultPrefValue(params->name);
+  }
+
+  pref_obj.type = GetPrefTypeName(value);
+  pref_obj.value = value->Clone();
+
+  return RespondNow(ArgumentList(
+      browser_os::GetPref::Results::Create(pref_obj)));
+}
+
+// BrowserOSSetPrefFunction
+ExtensionFunction::ResponseAction BrowserOSSetPrefFunction::Run() {
+  std::optional<browser_os::SetPref::Params> params =
+      browser_os::SetPref::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Security: only allow modifying browseros.* prefs
+  if (!params->name.starts_with("browseros.")) {
+    return RespondNow(Error("Only browseros.* preferences can be modified"));
+  }
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  PrefService* prefs = FindPrefService(params->name, profile);
+
+  if (!prefs) {
+    return RespondNow(Error("Preference not found: " + params->name));
+  }
+
+  prefs->Set(params->name, params->value);
+
+  return RespondNow(ArgumentList(
+      browser_os::SetPref::Results::Create(true)));
+}
+
+// BrowserOSGetAllPrefsFunction
+ExtensionFunction::ResponseAction BrowserOSGetAllPrefsFunction::Run() {
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  PrefService* profile_prefs = profile->GetPrefs();
+  PrefService* local_state = g_browser_process->local_state();
+
+  // Build a combined browseros prefs dict from both sources
+  base::Value::Dict combined_browseros;
+
+  // Lambda to merge browseros prefs from a PrefService
+  auto merge_prefs_from_service = [&](PrefService* prefs, const std::string& source_name) {
+    if (!prefs) {
+      return;
+    }
+
+    // Get all preference values (returns nested Dict structure)
+    base::Value::Dict pref_dict = prefs->GetPreferenceValues(
+        PrefService::INCLUDE_DEFAULTS);
+
+    // Look for "browseros" key in the top-level dict
+    const base::Value* browseros_value = pref_dict.Find("browseros");
+    if (browseros_value && browseros_value->is_dict()) {
+      // Merge this browseros dict into combined
+      combined_browseros.Merge(browseros_value->GetDict().Clone());
+      LOG(INFO) << "[browseros] GetAllPrefs: Found browseros.* prefs in " << source_name;
+    }
+  };
+
+  // Merge from both Local State and Profile prefs
+  merge_prefs_from_service(local_state, "local_state");
+  merge_prefs_from_service(profile_prefs, "profile_prefs");
+
+  // Create single PrefObject with the entire browseros dict
+  std::vector<browser_os::PrefObject> pref_objects;
+  browser_os::PrefObject pref_obj;
+  pref_obj.key = "browseros";
+  pref_obj.type = "dictionary";
+  pref_obj.value = base::Value(std::move(combined_browseros));
+  pref_objects.push_back(std::move(pref_obj));
+
+  return RespondNow(ArgumentList(
+      browser_os::GetAllPrefs::Results::Create(pref_objects)));
+}
+
+// BrowserOSLogMetricFunction
+ExtensionFunction::ResponseAction BrowserOSLogMetricFunction::Run() {
+  std::optional<browser_os::LogMetric::Params> params =
+      browser_os::LogMetric::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  const std::string& event_name = params->event_name;
+  
+  // Add "extension." prefix to distinguish from native events
+  std::string prefixed_event = "extension." + event_name;
+  
+  if (params->properties.has_value()) {
+    // The properties parameter is a Properties struct with additional_properties member
+    base::Value::Dict properties = params->properties->additional_properties.Clone();
+    
+    // Add extension ID as a property
+    properties.Set("extension_id", extension_id());
+    
+    browseros_metrics::BrowserOSMetrics::Log(prefixed_event, std::move(properties));
+  } else {
+    // No properties, just log with extension ID
+    browseros_metrics::BrowserOSMetrics::Log(prefixed_event, {
+      {"extension_id", base::Value(extension_id())}
+    });
+  }
+  
+  // Return void callback
+  return RespondNow(NoArguments());
+}
+
+// BrowserOSGetVersionNumberFunction
+ExtensionFunction::ResponseAction BrowserOSGetVersionNumberFunction::Run() {
+  // Get the version number from version_info
+  std::string version = std::string(version_info::GetVersionNumber());
+
+  return RespondNow(ArgumentList(
+      browser_os::GetVersionNumber::Results::Create(version)));
+}
+
+// BrowserOSGetBrowserosVersionNumberFunction
+ExtensionFunction::ResponseAction BrowserOSGetBrowserosVersionNumberFunction::Run() {
+  std::string version = std::string(version_info::GetBrowserOSVersionNumber());
+
+  return RespondNow(ArgumentList(
+      browser_os::GetBrowserosVersionNumber::Results::Create(version)));
+}
+
+// BrowserOSExecuteJavaScriptFunction
+ExtensionFunction::ResponseAction BrowserOSExecuteJavaScriptFunction::Run() {
+  std::optional<browser_os::ExecuteJavaScript::Params> params =
+      browser_os::ExecuteJavaScript::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    return RespondNow(Error(error_message));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Get the primary main frame
+  content::RenderFrameHost* rfh = web_contents->GetPrimaryMainFrame();
+  if (!rfh) {
+    return RespondNow(Error("No render frame"));
+  }
+  
+  LOG(INFO) << "[browseros] ExecuteJavaScript: Executing code in tab " << tab_info->tab_id;
+  
+  // Convert JavaScript code string to UTF16
+  std::u16string js_code = base::UTF8ToUTF16(params->code);
+  
+  // Execute the JavaScript code using ExecuteJavaScriptForTests
+  // This will return the result of the execution
+  rfh->ExecuteJavaScriptForTests(
+      js_code,
+      base::BindOnce(&BrowserOSExecuteJavaScriptFunction::OnJavaScriptExecuted,
+                     this),
+      /*honor_js_content_settings=*/false);
+  
+  return RespondLater();
+}
+
+void BrowserOSExecuteJavaScriptFunction::OnJavaScriptExecuted(base::Value result) {
+  LOG(INFO) << "[browseros] ExecuteJavaScript: Execution completed";
+
+  if (result.is_none()) {
+      // JavaScript returned undefined or execution failed
+      // Return an empty object instead of NONE to satisfy the validator
+      result = base::Value(base::Value::Type::DICT);
+  }
+  
+  // Return the result directly
+  Respond(ArgumentList(
+      browser_os::ExecuteJavaScript::Results::Create(result)));
+}
+
+// Implementation of BrowserOSClickCoordinatesFunction
+ExtensionFunction::ResponseAction BrowserOSClickCoordinatesFunction::Run() {
+  std::optional<browser_os::ClickCoordinates::Params> params =
+      browser_os::ClickCoordinates::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    LOG(ERROR) << "[browseros] ClickCoordinates: " << error_message;
+    browser_os::InteractionResponse response;
+    response.success = false;
+    return RespondNow(ArgumentList(
+        browser_os::ClickCoordinates::Results::Create(response)));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Create the click point from the coordinates
+  gfx::PointF click_point(params->x, params->y);
+  
+  LOG(INFO) << "[browseros] ClickCoordinates: Clicking at (" 
+            << params->x << ", " << params->y << ")";
+  
+  // Perform the click with change detection
+  bool success = ClickCoordinatesWithDetection(web_contents, click_point);
+  
+  // Prepare the response
+  browser_os::InteractionResponse response;
+  response.success = success;
+  
+  LOG(INFO) << "[browseros] ClickCoordinates: Result = " 
+            << (success ? "success" : "no change detected");
+  
+  return RespondNow(ArgumentList(
+      browser_os::ClickCoordinates::Results::Create(response)));
+}
+
+// Implementation of BrowserOSTypeAtCoordinatesFunction  
+ExtensionFunction::ResponseAction BrowserOSTypeAtCoordinatesFunction::Run() {
+  std::optional<browser_os::TypeAtCoordinates::Params> params =
+      browser_os::TypeAtCoordinates::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Get the target tab
+  std::string error_message;
+  auto tab_info = GetTabFromOptionalId(params->tab_id, browser_context(),
+                                       include_incognito_information(),
+                                       &error_message);
+  if (!tab_info) {
+    LOG(ERROR) << "[browseros] TypeAtCoordinates: " << error_message;
+    browser_os::InteractionResponse response;
+    response.success = false;
+    return RespondNow(ArgumentList(
+        browser_os::TypeAtCoordinates::Results::Create(response)));
+  }
+  
+  content::WebContents* web_contents = tab_info->web_contents;
+  
+  // Create the click point from the coordinates
+  gfx::PointF click_point(params->x, params->y);
+  
+  LOG(INFO) << "[browseros] TypeAtCoordinates: Clicking at (" 
+            << params->x << ", " << params->y << ") and typing: " << params->text;
+  
+  // Perform the click and type operation
+  bool success = TypeAtCoordinatesWithDetection(web_contents, click_point, params->text);
+  
+  // Prepare the response
+  browser_os::InteractionResponse response;
+  response.success = success;
+  
+  LOG(INFO) << "[browseros] TypeAtCoordinates: Result = " 
+            << (success ? "success" : "failed");
+  
+  return RespondNow(ArgumentList(
+      browser_os::TypeAtCoordinates::Results::Create(response)));
+}
+
+}  // namespace api
+}  // namespace extensions
