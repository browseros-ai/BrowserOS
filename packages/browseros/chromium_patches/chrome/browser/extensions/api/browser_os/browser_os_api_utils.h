diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h
new file mode 100644
index 0000000000000..f4fdcb73186cd
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h
@@ -0,0 +1,80 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_UTILS_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_UTILS_H_
+
+#include <optional>
+#include <string>
+#include <unordered_map>
+
+#include "base/memory/raw_ptr.h"
+#include "base/values.h"
+#include "chrome/common/extensions/api/blockbrowser.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_tree_id.h"
+#include "ui/gfx/geometry/rect_f.h"
+
+namespace content {
+class BrowserContext;
+class RenderWidgetHost;
+class WebContents;
+}  // namespace content
+
+namespace extensions {
+
+class WindowController;
+
+namespace api {
+
+// Result structure for tab retrieval
+struct TabInfo {
+  raw_ptr<content::WebContents> web_contents;
+  int tab_id;
+  
+  TabInfo(content::WebContents* wc, int id) 
+      : web_contents(wc), tab_id(id) {}
+};
+
+// Stores mapping information for a node
+struct NodeInfo {
+  NodeInfo();
+  ~NodeInfo();
+  NodeInfo(const NodeInfo&);
+  NodeInfo& operator=(const NodeInfo&);
+  NodeInfo(NodeInfo&&);
+  NodeInfo& operator=(NodeInfo&&);
+
+  int32_t ax_node_id;
+  ui::AXTreeID ax_tree_id;  // Tree ID for change detection
+  gfx::RectF bounds;  // Absolute bounds in CSS pixels
+  std::unordered_map<std::string, std::string> attributes;  // All computed attributes
+  blockbrowser::InteractiveNodeType node_type;  // Cached node type to avoid recomputation
+  bool in_viewport;  // Whether the node is currently visible in viewport
+};
+
+// Global node ID mappings storage
+std::unordered_map<int, std::unordered_map<uint32_t, NodeInfo>>& 
+GetNodeIdMappings();
+
+// Helper to get WebContents and tab ID from optional tab_id parameter
+// Returns nullptr if tab is not found, with error message set
+std::optional<TabInfo> GetTabFromOptionalId(
+    std::optional<int> tab_id_param,
+    content::BrowserContext* browser_context,
+    bool include_incognito_information,
+    std::string* error_message);
+
+// Helper to determine if a node is interactive (clickable/typable)
+blockbrowser::InteractiveNodeType GetInteractiveNodeType(
+    const ui::AXNodeData& node_data);
+
+// Helper to get the HTML tag name from AX role
+std::string GetTagFromRole(ax::mojom::Role role);
+
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_UTILS_H_
\ No newline at end of file
