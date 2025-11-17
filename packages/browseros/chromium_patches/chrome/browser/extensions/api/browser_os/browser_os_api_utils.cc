diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.cc b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.cc
new file mode 100644
index 0000000000000..0c3a060fc70de
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.cc
@@ -0,0 +1,167 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h"
+
+#include "base/hash/hash.h"
+#include "base/no_destructor.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/extensions/window_controller.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_finder.h"
+#include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/accessibility/ax_role_properties.h"
+
+namespace extensions {
+namespace api {
+
+// NodeInfo implementation
+NodeInfo::NodeInfo() : ax_node_id(0), ax_tree_id(), node_type(blockbrowser::InteractiveNodeType::kOther), in_viewport(false) {}
+NodeInfo::~NodeInfo() = default;
+NodeInfo::NodeInfo(const NodeInfo&) = default;
+NodeInfo& NodeInfo::operator=(const NodeInfo&) = default;
+NodeInfo::NodeInfo(NodeInfo&&) = default;
+NodeInfo& NodeInfo::operator=(NodeInfo&&) = default;
+
+// Global node ID mappings storage
+// Use NoDestructor to avoid exit-time destructor
+std::unordered_map<int, std::unordered_map<uint32_t, NodeInfo>>& 
+GetNodeIdMappings() {
+  static base::NoDestructor<std::unordered_map<int, std::unordered_map<uint32_t, NodeInfo>>> 
+      g_node_id_mappings;
+  return *g_node_id_mappings;
+}
+
+std::optional<TabInfo> GetTabFromOptionalId(
+    std::optional<int> tab_id_param,
+    content::BrowserContext* browser_context,
+    bool include_incognito_information,
+    std::string* error_message) {
+  content::WebContents* web_contents = nullptr;
+  int tab_id = -1;
+  
+  if (tab_id_param) {
+    // Get specific tab by ID
+    WindowController* controller = nullptr;
+    int tab_index = -1;
+    if (!ExtensionTabUtil::GetTabById(*tab_id_param, browser_context,
+                                      include_incognito_information,
+                                      &controller, &web_contents,
+                                      &tab_index)) {
+      if (error_message) {
+        *error_message = "Tab not found";
+      }
+      return std::nullopt;
+    }
+    tab_id = *tab_id_param;
+  } else {
+    // Get active tab
+    Browser* browser = chrome::FindLastActive();
+    if (!browser) {
+      if (error_message) {
+        *error_message = "No active browser";
+      }
+      return std::nullopt;
+    }
+
+    web_contents = browser->tab_strip_model()->GetActiveWebContents();
+    if (!web_contents) {
+      if (error_message) {
+        *error_message = "No active tab";
+      }
+      return std::nullopt;
+    }
+    tab_id = ExtensionTabUtil::GetTabId(web_contents);
+  }
+
+  return TabInfo(web_contents, tab_id);
+}
+
+// Helper to determine if a node is interactive (clickable/typeable/selectable)
+blockbrowser::InteractiveNodeType GetInteractiveNodeType(
+    const ui::AXNodeData& node_data) {
+  
+  // Skip invisible or ignored nodes early
+  if (node_data.IsInvisibleOrIgnored()) {
+    return blockbrowser::InteractiveNodeType::kOther;
+  }
+
+  // Use built-in IsTextField() and related methods for typeable elements
+  if (node_data.IsTextField() || 
+      node_data.IsPasswordField() || 
+      node_data.IsAtomicTextField() ||
+      node_data.IsNonAtomicTextField() ||
+      node_data.IsSpinnerTextField()) {
+    return blockbrowser::InteractiveNodeType::kTypeable;
+  }
+
+  // Use built-in IsSelectable() for selectable elements
+  if (node_data.IsSelectable()) {
+    return blockbrowser::InteractiveNodeType::kSelectable;
+  }
+  
+  // Use built-in IsClickable() method
+  if (node_data.IsClickable()) {
+    return blockbrowser::InteractiveNodeType::kClickable;
+  }
+  
+  // Additional check for combobox and list options which might not be caught by IsSelectable
+  using Role = ax::mojom::Role;
+  if (node_data.role == Role::kComboBoxSelect ||
+      node_data.role == Role::kComboBoxMenuButton ||
+      node_data.role == Role::kComboBoxGrouping ||
+      node_data.role == Role::kListBox ||
+      node_data.role == Role::kListBoxOption ||
+      node_data.role == Role::kMenuListOption ||
+      node_data.role == Role::kMenuItem ||
+      node_data.role == Role::kMenuItemCheckBox ||
+      node_data.role == Role::kMenuItemRadio) {
+    return blockbrowser::InteractiveNodeType::kSelectable;
+  }
+  
+  return blockbrowser::InteractiveNodeType::kOther;
+}
+
+// Helper to get the HTML tag name from AX role
+std::string GetTagFromRole(ax::mojom::Role role) {
+  switch (role) {
+    case ax::mojom::Role::kButton:
+      return "button";
+    case ax::mojom::Role::kLink:
+      return "a";
+    case ax::mojom::Role::kTextField:
+    case ax::mojom::Role::kSearchBox:
+      return "input";
+    case ax::mojom::Role::kTextFieldWithComboBox:
+      return "input";
+    case ax::mojom::Role::kComboBoxSelect:
+      return "select";
+    case ax::mojom::Role::kCheckBox:
+      return "input";
+    case ax::mojom::Role::kRadioButton:
+      return "input";
+    case ax::mojom::Role::kImage:
+      return "img";
+    case ax::mojom::Role::kHeading:
+      return "h1";  // Could be h1-h6
+    case ax::mojom::Role::kParagraph:
+      return "p";
+    case ax::mojom::Role::kListItem:
+      return "li";
+    case ax::mojom::Role::kList:
+      return "ul";
+    case ax::mojom::Role::kForm:
+      return "form";
+    case ax::mojom::Role::kTable:
+      return "table";
+    default:
+      return "div";
+  }
+}
+
+}  // namespace api
+}  // namespace extensions
