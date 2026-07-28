diff --git a/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.cc b/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.cc
new file mode 100644
index 0000000000000..4a801a5d847ea
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.cc
@@ -0,0 +1,231 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.h"
+
+#include <algorithm>
+#include <string>
+#include <unordered_map>
+
+#include "base/strings/string_util.h"
+#include "base/strings/utf_string_conversions.h"
+#include "ui/accessibility/ax_enums.mojom.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_role_properties.h"
+#include "ui/accessibility/ax_tree_update.h"
+
+namespace {
+
+// Forward declarations
+std::u16string GetNodeText(const ui::AXNodeData* node);
+void CleanupWhitespace(std::u16string& text);
+
+// Recursively extracts text from a node using DFS with semantic boundaries.
+// Stops recursion at headings, links, and images to prevent duplication.
+void ExtractNode(
+    int32_t node_id,
+    const std::unordered_map<int32_t, const ui::AXNodeData*>& node_map,
+    std::u16string& output,
+    int depth = 0) {
+
+  auto it = node_map.find(node_id);
+  if (it == node_map.end()) return;
+
+  const ui::AXNodeData* node = it->second;
+
+  // Skip invisible or ignored nodes but still process their children
+  if (node->IsInvisibleOrIgnored()) {
+    for (int32_t child_id : node->child_ids) {
+      ExtractNode(child_id, node_map, output, depth);
+    }
+    return;
+  }
+
+  // Handle different semantic elements (stop recursion at boundaries)
+
+  // NAVIGATION - Separate from main content
+  if (node->role == ax::mojom::Role::kNavigation ||
+      node->role == ax::mojom::Role::kBanner) {
+    // Add spacing before
+    if (!output.empty() && output.back() != u'\n') {
+      output += u"\n\n";
+    }
+
+    // Recurse to extract nav links
+    for (int32_t child_id : node->child_ids) {
+      ExtractNode(child_id, node_map, output, depth);
+    }
+
+    // Add spacing after to separate from content
+    output += u"\n\n";
+    return;
+  }
+
+  // HEADINGS - Extract and format as markdown
+  if (ui::IsHeading(node->role)) {
+    int level = 2;  // Default to h2
+    if (node->HasIntAttribute(ax::mojom::IntAttribute::kHierarchicalLevel)) {
+      level = node->GetIntAttribute(ax::mojom::IntAttribute::kHierarchicalLevel);
+      level = std::clamp(level, 1, 6);  // Ensure valid heading level
+    }
+
+    std::u16string text = GetNodeText(node);
+    if (!text.empty()) {
+      // Add newline if not at start
+      if (!output.empty() && output.back() != u'\n') {
+        output += u"\n\n";
+      }
+      // Add markdown heading
+      output += std::u16string(level, u'#') + u" " + text + u"\n\n";
+    }
+    return;  // Don't recurse into heading children
+  }
+
+  // LINKS - Extract text only (no URLs)
+  if (ui::IsLink(node->role)) {
+    std::u16string text = GetNodeText(node);
+    if (!text.empty()) {
+      output += text + u" ";
+    }
+    return;  // Don't recurse into link children
+  }
+
+  // IMAGES - Extract alt text
+  if (ui::IsImage(node->role)) {
+    std::u16string alt_text = GetNodeText(node);
+    if (!alt_text.empty()) {
+      output += u"[Image: " + alt_text + u"] ";
+    }
+    return;  // Don't recurse into image children
+  }
+
+  // TEXT NODES - Extract actual text content
+  if (ui::IsText(node->role)) {
+    std::u16string text = GetNodeText(node);
+    if (!text.empty()) {
+      // Add space if needed
+      if (!output.empty() && output.back() != u' ' && output.back() != u'\n') {
+        output += u" ";
+      }
+      output += text;
+    }
+    return;  // Terminal node, no children
+  }
+
+  // LIST container - Increase depth for nested structure
+  if (node->role == ax::mojom::Role::kList) {
+    for (int32_t child_id : node->child_ids) {
+      ExtractNode(child_id, node_map, output, depth + 1);
+    }
+    return;
+  }
+
+  // LIST ITEMS - Start new line with indentation
+  if (node->role == ax::mojom::Role::kListItem) {
+    // Start new line
+    if (!output.empty() && output.back() != u'\n') {
+      output += u"\n";
+    }
+
+    // Add indentation for nested items (only if depth > 0)
+    if (depth > 0) {
+      output += std::u16string(depth, u'\t');
+    }
+
+    // Extract children inline (same depth - they're siblings on same line)
+    for (int32_t child_id : node->child_ids) {
+      ExtractNode(child_id, node_map, output, depth);
+    }
+
+    return;  // Semantic boundary - don't let parent recurse again
+  }
+
+  // PARAGRAPHS - Add spacing
+  if (node->role == ax::mojom::Role::kParagraph) {
+    if (!output.empty() && output.back() != u'\n') {
+      output += u"\n\n";
+    }
+  }
+
+  // For all other container nodes, recurse to children
+  for (int32_t child_id : node->child_ids) {
+    ExtractNode(child_id, node_map, output, depth);
+  }
+
+  // Add spacing after certain block elements
+  if (node->role == ax::mojom::Role::kParagraph ||
+      node->role == ax::mojom::Role::kSection ||
+      node->role == ax::mojom::Role::kArticle) {
+    if (!output.empty() && output.back() != u'\n') {
+      output += u"\n\n";
+    }
+  }
+}
+
+// Helper to get text from a node (name or value)
+std::u16string GetNodeText(const ui::AXNodeData* node) {
+  std::string text;
+
+  // Try name attribute first (most common)
+  if (node->HasStringAttribute(ax::mojom::StringAttribute::kName)) {
+    text = node->GetStringAttribute(ax::mojom::StringAttribute::kName);
+  }
+  // Fall back to value attribute (for input fields)
+  else if (node->HasStringAttribute(ax::mojom::StringAttribute::kValue)) {
+    text = node->GetStringAttribute(ax::mojom::StringAttribute::kValue);
+  }
+
+  // Clean up the text
+  text = std::string(base::TrimWhitespaceASCII(text, base::TRIM_ALL));
+
+  // Convert to UTF16 and return
+  return base::UTF8ToUTF16(text);
+}
+
+// Clean up excessive whitespace in the final output
+void CleanupWhitespace(std::u16string& text) {
+  // Replace multiple spaces with single space
+  size_t pos = 0;
+  while ((pos = text.find(u"  ", pos)) != std::u16string::npos) {
+    text.replace(pos, 2, u" ");
+  }
+
+  // Replace multiple newlines (more than 2) with double newline
+  pos = 0;
+  while ((pos = text.find(u"\n\n\n", pos)) != std::u16string::npos) {
+    text.replace(pos, 3, u"\n\n");
+  }
+
+  // Trim trailing whitespace
+  while (!text.empty() && (text.back() == u' ' || text.back() == u'\n')) {
+    text.pop_back();
+  }
+}
+
+}  // namespace
+
+namespace side_panel {
+
+std::u16string BrowserOSSimplePageExtractor::ExtractStructuredText(
+    const ui::AXTreeUpdate& update) {
+  if (update.nodes.empty()) {
+    return u"";
+  }
+
+  // Build node map for O(1) lookup
+  std::unordered_map<int32_t, const ui::AXNodeData*> node_map;
+  for (const auto& node : update.nodes) {
+    node_map[node.id] = &node;
+  }
+
+  std::u16string output;
+  ExtractNode(update.root_id, node_map, output, -1);  // Start at depth -1
+
+  // Clean up extra whitespace
+  CleanupWhitespace(output);
+
+  return output;
+}
+
+}  // namespace side_panel
\ No newline at end of file
