diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.h
new file mode 100644
index 0000000000000..5c85cd73b26f3
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.h
@@ -0,0 +1,111 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_SNAPSHOT_PROCESSOR_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_SNAPSHOT_PROCESSOR_H_
+
+#include <cstdint>
+#include <string>
+#include <unordered_map>
+#include <vector>
+
+#include "base/functional/callback.h"
+#include "base/memory/raw_ptr.h"
+#include "chrome/common/extensions/api/blockbrowser.h"
+#include "ui/gfx/geometry/rect_f.h"
+
+namespace content {
+class WebContents;
+}  // namespace content
+
+namespace ui {
+class AXNode;
+class AXTree;
+struct AXNodeData;
+struct AXTreeUpdate;
+enum class AXCoordinateSystem;
+enum class AXClippingBehavior;
+}  // namespace ui
+
+namespace extensions {
+namespace api {
+
+// Result of snapshot processing
+struct SnapshotProcessingResult {
+  blockbrowser::InteractiveSnapshot snapshot;
+  int nodes_processed = 0;
+  int64_t processing_time_ms = 0;
+};
+
+// Processes accessibility trees into interactive snapshots with parallel processing
+class SnapshotProcessor {
+ public:
+  // Structure to hold data for a processed node
+  struct ProcessedNode {
+    ProcessedNode();
+    ProcessedNode(const ProcessedNode&);
+    ProcessedNode(ProcessedNode&&);
+    ProcessedNode& operator=(const ProcessedNode&);
+    ProcessedNode& operator=(ProcessedNode&&);
+    ~ProcessedNode();
+    
+    raw_ptr<const ui::AXNodeData> node_data;
+    uint32_t node_id;
+    blockbrowser::InteractiveNodeType node_type;
+    std::string name;
+    gfx::RectF absolute_bounds;
+    // All attributes stored as key-value pairs
+    std::unordered_map<std::string, std::string> attributes;
+  };
+
+  SnapshotProcessor() = default;
+  ~SnapshotProcessor() = default;
+
+  // Main processing function - handles all threading internally
+  // This function processes the accessibility tree into an interactive snapshot
+  // using parallel processing on the thread pool. Extracts viewport info from
+  // web_contents on UI thread before processing.
+  static void ProcessAccessibilityTree(
+      const ui::AXTreeUpdate& tree_update,
+      int tab_id,
+      uint32_t snapshot_id,
+      content::WebContents* web_contents,
+      base::OnceCallback<void(SnapshotProcessingResult)> callback);
+
+  // Process a batch of nodes (exposed for testing)
+  // The ax_tree is used to compute accurate bounds for each node
+  // device_scale_factor is used to convert physical pixels to CSS pixels
+  static std::vector<ProcessedNode> ProcessNodeBatch(
+      const std::vector<ui::AXNodeData>& nodes_to_process,
+      const std::unordered_map<int32_t, ui::AXNodeData>& node_map,
+      ui::AXTree* ax_tree,
+      uint32_t start_node_id,
+      float device_scale_factor = 1.0f);
+
+ private:
+  // Internal processing context
+  struct ProcessingContext;
+  
+  // Compute absolute bounds for a node using AXTree and convert to CSS pixels
+  // This implements the same logic as BrowserAccessibility::GetBoundsRect
+  // Returns bounds in CSS pixels by applying device_scale_factor
+  static gfx::RectF GetNodeBounds(ui::AXTree* tree, 
+                                   const ui::AXNode* node,
+                                   const ui::AXCoordinateSystem coordinate_system,
+                                   const ui::AXClippingBehavior clipping_behavior,
+                                   float device_scale_factor = 1.0f,
+                                   bool* out_offscreen = nullptr);
+  
+  // Batch processing callback
+  static void OnBatchProcessed(scoped_refptr<ProcessingContext> context,
+                               std::vector<ProcessedNode> batch_results);
+
+  SnapshotProcessor(const SnapshotProcessor&) = delete;
+  SnapshotProcessor& operator=(const SnapshotProcessor&) = delete;
+};
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_SNAPSHOT_PROCESSOR_H_
\ No newline at end of file
