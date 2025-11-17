diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.cc b/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.cc
new file mode 100644
index 0000000000000..885942336dcd6
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.cc
@@ -0,0 +1,648 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.h"
+
+#include <algorithm>
+#include <atomic>
+#include <cctype>
+#include <functional>
+#include <future>
+#include <memory>
+#include <queue>
+#include <sstream>
+#include <unordered_set>
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/ref_counted.h"
+#include "base/strings/string_util.h"
+#include "base/task/thread_pool.h"
+#include "base/time/time.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h"
+#include "content/public/browser/browser_thread.h"
+#include "content/public/browser/render_widget_host_view.h"
+#include "content/browser/renderer_host/render_widget_host_view_base.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/accessibility/ax_clipping_behavior.h"
+#include "ui/accessibility/ax_coordinate_system.h"
+#include "ui/accessibility/ax_enum_util.h"
+#include "ui/accessibility/ax_node.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_tree.h"
+#include "ui/accessibility/ax_tree_id.h"
+#include "ui/accessibility/ax_tree_update.h"
+#include "ui/gfx/geometry/rect.h"
+#include "ui/gfx/geometry/rect_conversions.h"
+#include "ui/gfx/geometry/rect_f.h"
+#include "ui/gfx/geometry/transform.h"
+
+namespace extensions {
+namespace api {
+
+// Static method to compute bounds for a node using AXTree and convert to CSS pixels
+// This implements the same logic as BrowserAccessibility::GetBoundsRect
+gfx::RectF SnapshotProcessor::GetNodeBounds(
+    ui::AXTree* tree,
+    const ui::AXNode* node,
+    const ui::AXCoordinateSystem coordinate_system,
+    const ui::AXClippingBehavior clipping_behavior,
+    float device_scale_factor,
+    bool* out_offscreen) {
+  if (!tree || !node) {
+    return gfx::RectF();
+  }
+  
+  // Start with empty bounds (same as GetBoundsRect does)
+  gfx::RectF bounds;
+  
+  // Apply RelativeToTreeBounds to get absolute bounds
+  const bool clip_bounds = clipping_behavior == ui::AXClippingBehavior::kClipped;
+  bool offscreen = false;
+  bounds = tree->RelativeToTreeBounds(node, bounds, &offscreen, clip_bounds);
+  
+  // Return offscreen status to caller
+  if (out_offscreen) {
+    *out_offscreen = offscreen;
+  }
+  
+  // Convert physical pixels to CSS pixels
+  if (device_scale_factor > 0.0f && device_scale_factor != 1.0f) {
+    bounds.set_x(bounds.x() / device_scale_factor);
+    bounds.set_y(bounds.y() / device_scale_factor);
+    bounds.set_width(bounds.width() / device_scale_factor);
+    bounds.set_height(bounds.height() / device_scale_factor);
+  }
+  
+  // Return bounds in CSS pixels
+  return bounds;
+}
+
+
+// ProcessedNode implementation
+SnapshotProcessor::ProcessedNode::ProcessedNode()
+    : node_data(nullptr), node_id(0) {}
+
+SnapshotProcessor::ProcessedNode::ProcessedNode(const ProcessedNode&) = default;
+SnapshotProcessor::ProcessedNode::ProcessedNode(ProcessedNode&&) = default;
+SnapshotProcessor::ProcessedNode& 
+SnapshotProcessor::ProcessedNode::operator=(const ProcessedNode&) = default;
+SnapshotProcessor::ProcessedNode& 
+SnapshotProcessor::ProcessedNode::operator=(ProcessedNode&&) = default;
+SnapshotProcessor::ProcessedNode::~ProcessedNode() = default;
+
+
+namespace {
+
+// Helper to sanitize strings to ensure valid UTF-8 by keeping only printable ASCII
+std::string SanitizeStringForOutput(const std::string& input) {
+  std::string output;
+  output.reserve(input.size());
+  
+  for (char c : input) {
+    // Only include printable ASCII and whitespace
+    if ((c >= 32 && c <= 126) || c == '\t' || c == '\n') {
+      output.push_back(c);
+    } else {
+      output.push_back(' ');  // Replace non-printable with space
+    }
+  }
+  
+  return output;
+}
+
+// Helper to determine if a node should be skipped for the interactive snapshot
+bool ShouldSkipNode(const ui::AXNodeData& node_data) {
+  // Skip invisible or ignored nodes
+  if (node_data.IsInvisibleOrIgnored()) {
+    return true;
+  }
+  
+  // Get the interactive type and skip if it's not interactive
+  blockbrowser::InteractiveNodeType node_type = GetInteractiveNodeType(node_data);
+  if (node_type == blockbrowser::InteractiveNodeType::kOther) {
+    return true;
+  }
+  
+  return false;
+}
+
+}  // namespace
+
+// Internal structure for managing async processing
+struct SnapshotProcessor::ProcessingContext 
+    : public base::RefCountedThreadSafe<ProcessingContext> {
+  blockbrowser::InteractiveSnapshot snapshot;
+  std::unordered_map<int32_t, ui::AXNodeData> node_map;
+  std::unordered_map<int32_t, int32_t> parent_map;  // child_id -> parent_id  
+  std::unordered_map<int32_t, std::vector<int32_t>> children_map;  // parent_id -> child_ids
+  std::unique_ptr<ui::AXTree> ax_tree;  // AXTree for computing accurate bounds
+  int tab_id;
+  ui::AXTreeID tree_id;  // Tree ID for change detection
+  float device_scale_factor = 1.0f;  // For converting physical to CSS pixels
+  gfx::Size viewport_size;  // For visibility checks
+  base::TimeTicks start_time;
+  size_t total_nodes;
+  size_t processed_batches;
+  size_t total_batches;
+  base::OnceCallback<void(SnapshotProcessingResult)> callback;
+  
+ private:
+  friend class base::RefCountedThreadSafe<ProcessingContext>;
+  ~ProcessingContext() = default;
+};
+
+// Helper to collect text from a node's subtree
+std::string CollectTextFromNode(
+    int32_t node_id,
+    const std::unordered_map<int32_t, ui::AXNodeData>& node_map,
+    int max_chars = 200) {
+  
+  auto node_it = node_map.find(node_id);
+  if (node_it == node_map.end()) {
+    return "";
+  }
+  
+  std::vector<std::string> text_parts;
+  
+  // BFS to collect text from this node and its children
+  std::queue<int32_t> queue;
+  queue.push(node_id);
+  int chars_collected = 0;
+  
+  while (!queue.empty() && chars_collected < max_chars) {
+    int32_t current_id = queue.front();
+    queue.pop();
+    
+    auto current_it = node_map.find(current_id);
+    if (current_it == node_map.end()) continue;
+    
+    const ui::AXNodeData& current = current_it->second;
+    
+    // Collect text from this node
+    if (current.HasStringAttribute(ax::mojom::StringAttribute::kName)) {
+      std::string text = current.GetStringAttribute(ax::mojom::StringAttribute::kName);
+      text = std::string(base::TrimWhitespaceASCII(text, base::TRIM_ALL));
+      if (!text.empty()) {
+        std::string clean_text = SanitizeStringForOutput(text);
+        if (!clean_text.empty()) {
+          text_parts.push_back(clean_text);
+          chars_collected += clean_text.length();
+        }
+      }
+    }
+    
+    // Add children to queue
+    for (int32_t child_id : current.child_ids) {
+      queue.push(child_id);
+    }
+  }
+  
+  std::string result = base::JoinString(text_parts, " ");
+  if (result.length() > static_cast<size_t>(max_chars)) {
+    result = result.substr(0, max_chars - 3) + "...";
+  }
+  return result;
+}
+
+// Helper to build path using offset_container_id and return depth
+std::pair<std::string, int> BuildPathAndDepth(
+    int32_t node_id,
+    const std::unordered_map<int32_t, ui::AXNodeData>& node_map) {
+  
+  std::vector<std::string> path_parts;
+  int32_t current_id = node_id;
+  int depth = 0;
+  const int max_depth = 10;
+  
+  while (current_id >= 0 && depth < max_depth) {
+    auto node_it = node_map.find(current_id);
+    if (node_it == node_map.end()) break;
+    
+    const ui::AXNodeData& node = node_it->second;
+    
+    // Just append the role
+    path_parts.push_back(ui::ToString(node.role));
+    
+    // Move to offset container
+    current_id = node.relative_bounds.offset_container_id;
+    depth++;
+  }
+  
+  // Reverse to get top-down path
+  std::reverse(path_parts.begin(), path_parts.end());
+  return std::make_pair(base::JoinString(path_parts, " > "), depth);
+}
+
+// Helper to populate all attributes for a node
+void PopulateNodeAttributes(
+    const ui::AXNodeData& node_data,
+    std::unordered_map<std::string, std::string>& attributes) {
+  
+  // Add role as string
+  attributes["role"] = ui::ToString(node_data.role);
+  
+  // Add value attribute for inputs
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kValue)) {
+    std::string value = node_data.GetStringAttribute(ax::mojom::StringAttribute::kValue);
+    attributes["value"] = SanitizeStringForOutput(value);
+  }
+  
+  // Add HTML tag if available
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kHtmlTag)) {
+    attributes["html-tag"] = node_data.GetStringAttribute(ax::mojom::StringAttribute::kHtmlTag);
+  }
+  
+  // Add role description
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kRoleDescription)) {
+    std::string role_desc = node_data.GetStringAttribute(ax::mojom::StringAttribute::kRoleDescription);
+    attributes["role-description"] = SanitizeStringForOutput(role_desc);
+  }
+  
+  // Add input type
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kInputType)) {
+    std::string input_type = node_data.GetStringAttribute(ax::mojom::StringAttribute::kInputType);
+    attributes["input-type"] = SanitizeStringForOutput(input_type);
+  }
+  
+  // Add tooltip
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kTooltip)) {
+    std::string tooltip = node_data.GetStringAttribute(ax::mojom::StringAttribute::kTooltip);
+    attributes["tooltip"] = SanitizeStringForOutput(tooltip);
+  }
+  
+  // Add placeholder for input fields
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kPlaceholder)) {
+    std::string placeholder = node_data.GetStringAttribute(ax::mojom::StringAttribute::kPlaceholder);
+    attributes["placeholder"] = SanitizeStringForOutput(placeholder);
+  }
+  
+  // Add description for more context
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kDescription)) {
+    std::string description = node_data.GetStringAttribute(ax::mojom::StringAttribute::kDescription);
+    attributes["description"] = SanitizeStringForOutput(description);
+  }
+  
+  // Add URL for links
+  // if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kUrl)) {
+  //   std::string url = node_data.GetStringAttribute(ax::mojom::StringAttribute::kUrl);
+  //   attributes["url"] = SanitizeStringForOutput(url);
+  // }
+  
+  // Add checked state description
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kCheckedStateDescription)) {
+    std::string checked_desc = node_data.GetStringAttribute(ax::mojom::StringAttribute::kCheckedStateDescription);
+    attributes["checked-state"] = SanitizeStringForOutput(checked_desc);
+  }
+  
+  // Add autocomplete hint
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kAutoComplete)) {
+    std::string autocomplete = node_data.GetStringAttribute(ax::mojom::StringAttribute::kAutoComplete);
+    attributes["autocomplete"] = SanitizeStringForOutput(autocomplete);
+  }
+  
+  // Add HTML ID for form associations
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kHtmlId)) {
+    std::string html_id = node_data.GetStringAttribute(ax::mojom::StringAttribute::kHtmlId);
+    attributes["id"] = SanitizeStringForOutput(html_id);
+  }
+  
+  // Add HTML class names
+  if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kClassName)) {
+    std::string class_name = node_data.GetStringAttribute(ax::mojom::StringAttribute::kClassName);
+    attributes["class"] = SanitizeStringForOutput(class_name);
+  }
+}
+
+// Process a batch of nodes
+std::vector<SnapshotProcessor::ProcessedNode> SnapshotProcessor::ProcessNodeBatch(
+    const std::vector<ui::AXNodeData>& nodes_to_process,
+    const std::unordered_map<int32_t, ui::AXNodeData>& node_map,
+    ui::AXTree* ax_tree,
+    uint32_t start_node_id,
+    float device_scale_factor) {
+  std::vector<ProcessedNode> results;
+  results.reserve(nodes_to_process.size());
+  
+  uint32_t current_node_id = start_node_id;
+  
+  for (const auto& node_data : nodes_to_process) {
+    // Skip invisible, ignored, or non-interactive elements
+    if (ShouldSkipNode(node_data)) {
+      continue;
+    }
+    
+    // Double-check invisibility (already done in ShouldSkipNode, but being explicit)
+    if (node_data.IsInvisibleOrIgnored()) {
+      continue;
+    }
+    
+    // Get the interactive node type
+    blockbrowser::InteractiveNodeType node_type = GetInteractiveNodeType(node_data);
+    
+    ProcessedNode data;
+    data.node_data = &node_data;
+    data.node_id = current_node_id++;
+    data.node_type = node_type;
+    
+    // Get accessible name
+    if (node_data.HasStringAttribute(ax::mojom::StringAttribute::kName)) {
+      std::string name = node_data.GetStringAttribute(ax::mojom::StringAttribute::kName);
+      data.name = SanitizeStringForOutput(name);
+    }
+
+    // Compute bounds using AXTree
+    bool is_offscreen = false;
+    if (ax_tree) {
+      ui::AXNode* ax_node = ax_tree->GetFromId(node_data.id);
+      if (ax_node) {
+        // GetNodeBounds now returns CSS pixels directly
+        data.absolute_bounds = GetNodeBounds(
+            ax_tree, 
+            ax_node,
+            ui::AXCoordinateSystem::kFrame,
+            // Use clipped bounds so the center lies within the visible area of
+            // scrolled/clip containers. This matches how clicks should target
+            // on-screen rects.
+            ui::AXClippingBehavior::kClipped,
+            device_scale_factor,  // Pass DSF for CSS pixel conversion
+            &is_offscreen);
+        
+        VLOG(3) << "[browseros] Node " << node_data.id 
+                << " CSS bounds: " << data.absolute_bounds.ToString()
+                << " offscreen: " << is_offscreen;
+      } else {
+        // Node not found in AXTree, skip bounds computation
+        VLOG(3) << "[browseros] Node " << node_data.id 
+                << " not found in AXTree, skipping bounds";
+      }
+    } else {
+      // No AXTree available
+      LOG(WARNING) << "[browseros] No AXTree available for bounds computation";
+    }
+    
+    // Populate all attributes using helper function
+    PopulateNodeAttributes(node_data, data.attributes);
+    
+    // Add context from parent node
+    int32_t parent_id = node_data.relative_bounds.offset_container_id;
+    if (parent_id >= 0) {
+      std::string context = CollectTextFromNode(parent_id, node_map, 200);
+      if (!context.empty()) {
+        data.attributes["context"] = context;
+      }
+    }
+    
+    // Add path and depth using offset_container_id chain
+    auto [path, depth] = BuildPathAndDepth(node_data.id, node_map);
+    if (!path.empty()) {
+      data.attributes["path"] = path;
+    }
+    data.attributes["depth"] = std::to_string(depth);
+    
+    // Set viewport status based on offscreen flag
+    // Note: offscreen=false means the node IS in viewport (at least partially visible)
+    // offscreen=true means the node is NOT in viewport (completely hidden)
+    data.attributes["in_viewport"] = is_offscreen ? "false" : "true";
+    
+    results.push_back(std::move(data));
+  }
+  
+  return results;
+}
+
+// Helper to handle batch processing results
+void SnapshotProcessor::OnBatchProcessed(
+    scoped_refptr<ProcessingContext> context,
+    std::vector<ProcessedNode> batch_results) {
+  // Process batch results
+  for (const auto& node_data : batch_results) {
+    // Store mapping from our nodeId to AX node ID, bounds, and attributes
+    NodeInfo info;
+    info.ax_node_id = node_data.node_data->id;
+    info.ax_tree_id = context->tree_id;  // Store tree ID for change detection
+    info.bounds = node_data.absolute_bounds;
+    info.attributes = node_data.attributes;  // Store all computed attributes
+    info.node_type = node_data.node_type;  // Store node type for efficient filtering
+    // Extract in_viewport from attributes (stored as "true"/"false" string)
+    auto viewport_it = node_data.attributes.find("in_viewport");
+    info.in_viewport = (viewport_it != node_data.attributes.end() && viewport_it->second == "true");
+    GetNodeIdMappings()[context->tab_id][node_data.node_id] = info;
+    
+    // Log the mapping for debugging
+    VLOG(2) << "Node ID Mapping: Interactive nodeId=" << node_data.node_id 
+            << " -> AX node ID=" << info.ax_node_id 
+            << " (name: " << node_data.name << ")";
+    
+    // Create interactive node
+    blockbrowser::InteractiveNode interactive_node;
+    interactive_node.node_id = node_data.node_id;
+    interactive_node.type = node_data.node_type;
+    interactive_node.name = node_data.name;
+    
+    // Set the bounding rectangle
+    blockbrowser::Rect rect;
+    rect.x = node_data.absolute_bounds.x();
+    rect.y = node_data.absolute_bounds.y();
+    rect.width = node_data.absolute_bounds.width();
+    rect.height = node_data.absolute_bounds.height();
+    interactive_node.rect = std::move(rect);
+    
+    // Create attributes dictionary by iterating over all key-value pairs
+    if (!node_data.attributes.empty()) {
+      blockbrowser::InteractiveNode::Attributes attributes;
+      
+      // Iterate over all attributes and add them to the dictionary
+      for (const auto& [key, value] : node_data.attributes) {
+        attributes.additional_properties.Set(key, value);
+      }
+      
+      interactive_node.attributes = std::move(attributes);
+    }
+    
+    context->snapshot.elements.push_back(std::move(interactive_node));
+  }
+  
+  context->processed_batches++;
+  
+  // Check if all batches are complete
+  if (context->processed_batches == context->total_batches) {
+    // Sort elements by node_id to maintain consistent ordering
+    std::sort(context->snapshot.elements.begin(), 
+              context->snapshot.elements.end(),
+              [](const blockbrowser::InteractiveNode& a, 
+                 const blockbrowser::InteractiveNode& b) {
+                return a.node_id < b.node_id;
+              });
+
+    // Leave hierarchical_structure empty for now as requested
+    context->snapshot.hierarchical_structure = "";
+
+    base::TimeDelta processing_time = base::TimeTicks::Now() - context->start_time;
+    LOG(INFO) << "[PERF] Interactive snapshot processed in " 
+              << processing_time.InMilliseconds() << " ms"
+              << " (nodes: " << context->snapshot.elements.size() << ")";
+
+    // Set processing time in the snapshot
+    context->snapshot.processing_time_ms = processing_time.InMilliseconds();
+
+    SnapshotProcessingResult result;
+    result.snapshot = std::move(context->snapshot);
+    result.nodes_processed = context->total_nodes;
+    result.processing_time_ms = processing_time.InMilliseconds();
+    
+    // Run callback (context will be deleted when last ref is released)
+    std::move(context->callback).Run(std::move(result));
+  }
+}
+
+// Main processing function
+// Helper function to extract viewport info from WebContents
+// Returns viewport size and device scale factor
+static std::pair<gfx::Size, float> ExtractViewportInfo(
+    content::WebContents* web_contents) {
+  gfx::Size viewport_size;
+  float device_scale_factor = 1.0f;
+  
+  if (web_contents) {
+    if (auto* rwhv = web_contents->GetRenderWidgetHostView()) {
+      viewport_size = rwhv->GetVisibleViewportSize();
+      
+      // Get device scale factor for CSS pixel conversion
+      if (auto* rwhv_base = 
+          static_cast<content::RenderWidgetHostViewBase*>(rwhv)) {
+        device_scale_factor = rwhv_base->GetDeviceScaleFactor();
+      }
+    }
+  }
+  
+  LOG(INFO) << "[browseros] Viewport: " << viewport_size.ToString() 
+            << ", DSF: " << device_scale_factor;
+  
+  return {viewport_size, device_scale_factor};
+}
+
+void SnapshotProcessor::ProcessAccessibilityTree(
+    const ui::AXTreeUpdate& tree_update,
+    int tab_id,
+    uint32_t snapshot_id,
+    content::WebContents* web_contents,
+    base::OnceCallback<void(SnapshotProcessingResult)> callback) {
+  base::TimeTicks start_time = base::TimeTicks::Now();
+  
+  // Extract viewport info from WebContents on UI thread
+  auto [viewport_size, device_scale_factor] = ExtractViewportInfo(web_contents);
+  
+  // Build node ID map, parent map and children map for efficient lookup
+  std::unordered_map<int32_t, ui::AXNodeData> node_map;
+  std::unordered_map<int32_t, int32_t> parent_map;
+  std::unordered_map<int32_t, std::vector<int32_t>> children_map;
+  
+  for (const auto& node : tree_update.nodes) {
+    node_map[node.id] = node;
+    // Build parent and children relationships
+    for (int32_t child_id : node.child_ids) {
+      parent_map[child_id] = node.id;
+      children_map[node.id].push_back(child_id);
+    }
+  }
+  
+  // Clear previous mappings for this tab
+  GetNodeIdMappings()[tab_id].clear();
+
+  // Create an AXTree from the tree update for accurate bounds computation
+  std::unique_ptr<ui::AXTree> ax_tree = std::make_unique<ui::AXTree>(tree_update);
+  
+  if (!ax_tree) {
+    LOG(ERROR) << "[browseros] Failed to create AXTree from update";
+    SnapshotProcessingResult result;
+    result.nodes_processed = 0;
+    result.processing_time_ms = 0;
+    std::move(callback).Run(std::move(result));
+    return;
+  }
+  
+  LOG(INFO) << "[browseros] Created AXTree with " << tree_update.nodes.size() 
+            << " nodes for bounds computation";
+  
+  // Prepare processing context using RefCounted
+  auto context = base::MakeRefCounted<ProcessingContext>();
+  context->snapshot.snapshot_id = snapshot_id;
+  context->snapshot.timestamp = base::Time::Now().InMillisecondsFSinceUnixEpoch();
+  context->tab_id = tab_id;
+  context->node_map = std::move(node_map);
+  context->parent_map = std::move(parent_map); 
+  context->children_map = std::move(children_map);
+  context->ax_tree = std::move(ax_tree);  // Store AXTree for bounds computation
+  context->device_scale_factor = device_scale_factor;  // For CSS pixel conversion
+  context->viewport_size = viewport_size;  // For visibility checks
+  context->start_time = start_time;
+  
+  // Store the tree ID for change detection
+  if (tree_update.has_tree_data) {
+    context->tree_id = tree_update.tree_data.tree_id;
+  }
+  
+  // Viewport size is passed in but not currently used for viewport bounds calculation
+  // TODO: Implement proper viewport detection if needed
+  context->callback = std::move(callback);
+  context->processed_batches = 0;
+  
+  // Collect all nodes to process and filter
+  std::vector<ui::AXNodeData> nodes_to_process;
+  for (const auto& node : tree_update.nodes) {
+    // Skip invisible, ignored, or non-interactive nodes
+    if (ShouldSkipNode(node)) {
+      continue;
+    }
+    nodes_to_process.push_back(node);
+  }
+  
+  context->total_nodes = nodes_to_process.size();
+  
+  // Handle empty case
+  if (nodes_to_process.empty()) {
+    base::TimeDelta processing_time = base::TimeTicks::Now() - start_time;
+    context->snapshot.processing_time_ms = processing_time.InMilliseconds();
+    
+    SnapshotProcessingResult result;
+    result.snapshot = std::move(context->snapshot);
+    result.nodes_processed = 0;
+    result.processing_time_ms = processing_time.InMilliseconds();
+    std::move(context->callback).Run(std::move(result));
+    return;
+  }
+  
+  // Process nodes in batches using ThreadPool
+  const size_t batch_size = 100;  // Process 100 nodes per batch
+  size_t num_batches = (nodes_to_process.size() + batch_size - 1) / batch_size;
+  context->total_batches = num_batches;
+  
+  for (size_t i = 0; i < nodes_to_process.size(); i += batch_size) {
+    size_t end = std::min(i + batch_size, nodes_to_process.size());
+    std::vector<ui::AXNodeData> batch(
+        std::make_move_iterator(nodes_to_process.begin() + i),
+        std::make_move_iterator(nodes_to_process.begin() + end));
+    uint32_t start_node_id = i + 1;  // Node IDs start at 1
+    
+    // Post task to ThreadPool and handle result on UI thread
+    base::ThreadPool::PostTaskAndReplyWithResult(
+        FROM_HERE,
+        {base::TaskPriority::USER_VISIBLE},
+        base::BindOnce(&SnapshotProcessor::ProcessNodeBatch, 
+                       std::move(batch), 
+                       context->node_map,
+                       context->ax_tree.get(),  // Pass AXTree pointer for bounds computation
+                       start_node_id,
+                       context->device_scale_factor),  // Pass DSF for CSS pixel conversion
+        base::BindOnce(&SnapshotProcessor::OnBatchProcessed,
+                       context));
+  }
+}
+
+
+}  // namespace api
+}  // namespace extensions
