diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.cc b/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.cc
new file mode 100644
index 0000000000000..4166f1a38737c
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.cc
@@ -0,0 +1,247 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.h"
+
+#include <algorithm>
+
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "ui/accessibility/ax_enum_util.h"
+#include "ui/accessibility/ax_enums.mojom.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_role_properties.h"
+#include "ui/gfx/geometry/rect.h"
+#include "ui/gfx/geometry/rect_conversions.h"
+
+namespace extensions {
+namespace api {
+
+namespace {
+
+// Clean whitespace from text
+std::string CleanText(const std::string& text) {
+  std::string cleaned = std::string(base::TrimWhitespaceASCII(text, base::TRIM_ALL));
+
+  // Replace multiple spaces with single space
+  std::string result;
+  bool prev_space = false;
+  for (char c : cleaned) {
+    if (std::isspace(c)) {
+      if (!prev_space) {
+        result += ' ';
+        prev_space = true;
+      }
+    } else {
+      result += c;
+      prev_space = false;
+    }
+  }
+
+  return result;
+}
+
+}  // namespace
+
+// static
+std::vector<blockbrowser::ContentItem> ContentProcessor::ExtractPageContent(
+    const ui::AXTreeUpdate& tree_update) {
+
+  std::vector<blockbrowser::ContentItem> items;
+
+  if (tree_update.nodes.empty()) {
+    LOG(INFO) << "browseros: ExtractPageContent - tree is empty";
+    return items;
+  }
+
+  LOG(INFO) << "browseros: ExtractPageContent - processing " << tree_update.nodes.size() << " nodes";
+
+  // Build node map for O(1) lookup
+  std::unordered_map<int32_t, ui::AXNodeData> node_map;
+  for (const auto& node : tree_update.nodes) {
+    node_map[node.id] = node;
+  }
+
+  // Start DFS from root
+  TraverseDFS(tree_update.root_id, node_map, items);
+
+  LOG(INFO) << "browseros: ExtractPageContent - extracted " << items.size() << " items";
+
+  return items;
+}
+
+// static
+void ContentProcessor::TraverseDFS(
+    int32_t node_id,
+    const std::unordered_map<int32_t, ui::AXNodeData>& node_map,
+    std::vector<blockbrowser::ContentItem>& items) {
+
+  auto it = node_map.find(node_id);
+  if (it == node_map.end()) {
+    return;
+  }
+
+  const ui::AXNodeData& node = it->second;
+
+  // Skip extracting from ignored nodes, but still recurse to children
+  if (node.IsIgnored()) {
+    for (int32_t child_id : node.child_ids) {
+      TraverseDFS(child_id, node_map, items);
+    }
+    return;
+  }
+
+  // Extract content at semantic boundaries
+  // Don't recurse into these - their children are just formatting
+
+  if (ui::IsHeading(node.role)) {
+    items.push_back(ExtractHeading(node));
+    return;
+  }
+
+  if (ui::IsLink(node.role)) {
+    items.push_back(ExtractLink(node));
+    return;
+  }
+
+  if (ui::IsImage(node.role)) {
+    items.push_back(ExtractImage(node));
+    return;
+  }
+
+  if (node.role == ax::mojom::Role::kVideo) {
+    items.push_back(ExtractVideo(node));
+    return;
+  }
+
+  if (ui::IsText(node.role)) {
+    // Extract text content
+    auto item = ExtractText(node);
+    if (item.text.has_value() && !item.text->empty()) {
+      items.push_back(std::move(item));
+    }
+    return;
+  }
+
+  // For container nodes (divs, sections, etc.), recurse to children
+  for (int32_t child_id : node.child_ids) {
+    TraverseDFS(child_id, node_map, items);
+  }
+}
+
+// static
+blockbrowser::ContentItem ContentProcessor::ExtractHeading(
+    const ui::AXNodeData& node) {
+  blockbrowser::ContentItem item;
+  item.type = blockbrowser::ContentItemType::kHeading;
+
+  std::string name = GetAccessibleName(node);
+  if (!name.empty()) {
+    item.text = CleanText(name);
+  }
+
+  // Get heading level from hierarchical level attribute
+  if (node.HasIntAttribute(ax::mojom::IntAttribute::kHierarchicalLevel)) {
+    int level = node.GetIntAttribute(ax::mojom::IntAttribute::kHierarchicalLevel);
+    item.level = std::clamp(level, 1, 6);
+  } else {
+    // Default to level 2 if not specified
+    item.level = 2;
+  }
+
+  return item;
+}
+
+// static
+blockbrowser::ContentItem ContentProcessor::ExtractText(
+    const ui::AXNodeData& node) {
+  blockbrowser::ContentItem item;
+  item.type = blockbrowser::ContentItemType::kText;
+
+  std::string name = GetAccessibleName(node);
+  if (!name.empty()) {
+    item.text = CleanText(name);
+  }
+
+  return item;
+}
+
+// static
+blockbrowser::ContentItem ContentProcessor::ExtractLink(
+    const ui::AXNodeData& node) {
+  blockbrowser::ContentItem item;
+  item.type = blockbrowser::ContentItemType::kLink;
+
+  std::string name = GetAccessibleName(node);
+  if (!name.empty()) {
+    item.text = CleanText(name);
+  }
+
+  // Get URL from url attribute
+  if (node.HasStringAttribute(ax::mojom::StringAttribute::kUrl)) {
+    item.url = node.GetStringAttribute(ax::mojom::StringAttribute::kUrl);
+  }
+
+  return item;
+}
+
+// static
+blockbrowser::ContentItem ContentProcessor::ExtractImage(
+    const ui::AXNodeData& node) {
+  blockbrowser::ContentItem item;
+  item.type = blockbrowser::ContentItemType::kImage;
+
+  // Get alt text from name
+  std::string name = GetAccessibleName(node);
+  if (!name.empty()) {
+    item.alt = CleanText(name);
+  }
+
+  // Get image URL
+  if (node.HasStringAttribute(ax::mojom::StringAttribute::kUrl)) {
+    item.url = node.GetStringAttribute(ax::mojom::StringAttribute::kUrl);
+  } else if (node.HasStringAttribute(ax::mojom::StringAttribute::kImageDataUrl)) {
+    item.url = node.GetStringAttribute(ax::mojom::StringAttribute::kImageDataUrl);
+  }
+
+  return item;
+}
+
+// static
+blockbrowser::ContentItem ContentProcessor::ExtractVideo(
+    const ui::AXNodeData& node) {
+  blockbrowser::ContentItem item;
+  item.type = blockbrowser::ContentItemType::kVideo;
+
+  // Get video title from name
+  std::string name = GetAccessibleName(node);
+  if (!name.empty()) {
+    item.alt = CleanText(name);
+  }
+
+  // Get video URL
+  if (node.HasStringAttribute(ax::mojom::StringAttribute::kUrl)) {
+    item.url = node.GetStringAttribute(ax::mojom::StringAttribute::kUrl);
+  }
+
+  return item;
+}
+
+// static
+std::string ContentProcessor::GetAccessibleName(const ui::AXNodeData& node) {
+  // Try name attribute first
+  if (node.HasStringAttribute(ax::mojom::StringAttribute::kName)) {
+    return node.GetStringAttribute(ax::mojom::StringAttribute::kName);
+  }
+
+  // Fall back to value attribute
+  if (node.HasStringAttribute(ax::mojom::StringAttribute::kValue)) {
+    return node.GetStringAttribute(ax::mojom::StringAttribute::kValue);
+  }
+
+  return "";
+}
+
+}  // namespace api
+}  // namespace extensions
