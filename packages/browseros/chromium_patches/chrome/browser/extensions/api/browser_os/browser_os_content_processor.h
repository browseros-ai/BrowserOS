diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.h
new file mode 100644
index 0000000000000..472c7b114fe97
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.h
@@ -0,0 +1,57 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CONTENT_PROCESSOR_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CONTENT_PROCESSOR_H_
+
+#include <string>
+#include <unordered_map>
+#include <vector>
+
+#include "chrome/common/extensions/api/blockbrowser.h"
+#include "ui/accessibility/ax_tree_update.h"
+#include "ui/gfx/geometry/size.h"
+
+namespace ui {
+struct AXNodeData;
+}  // namespace ui
+
+namespace extensions {
+namespace api {
+
+// Extracts page content (headings, text, links, images, videos) from
+// accessibility tree in document order using depth-first traversal.
+class ContentProcessor {
+ public:
+  ContentProcessor() = delete;
+  ContentProcessor(const ContentProcessor&) = delete;
+  ContentProcessor& operator=(const ContentProcessor&) = delete;
+
+  // Extracts page content in document order.
+  // Returns content items preserving the order they appear in the document.
+  static std::vector<blockbrowser::ContentItem> ExtractPageContent(
+      const ui::AXTreeUpdate& tree_update);
+
+ private:
+  // DFS traversal to extract content in document order
+  static void TraverseDFS(
+      int32_t node_id,
+      const std::unordered_map<int32_t, ui::AXNodeData>& node_map,
+      std::vector<blockbrowser::ContentItem>& items);
+
+  // Content extraction helpers
+  static blockbrowser::ContentItem ExtractHeading(const ui::AXNodeData& node);
+  static blockbrowser::ContentItem ExtractText(const ui::AXNodeData& node);
+  static blockbrowser::ContentItem ExtractLink(const ui::AXNodeData& node);
+  static blockbrowser::ContentItem ExtractImage(const ui::AXNodeData& node);
+  static blockbrowser::ContentItem ExtractVideo(const ui::AXNodeData& node);
+
+  // Get accessible name from node
+  static std::string GetAccessibleName(const ui::AXNodeData& node);
+};
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_CONTENT_PROCESSOR_H_
