diff --git a/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.h b/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.h
new file mode 100644
index 0000000000000..1fed9958dcfcb
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/browseros_simple_page_extractor.h
@@ -0,0 +1,71 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_BROWSEROS_SIMPLE_PAGE_EXTRACTOR_H_
+#define CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_BROWSEROS_SIMPLE_PAGE_EXTRACTOR_H_
+
+#include <string>
+
+namespace ui {
+struct AXTreeUpdate;
+}  // namespace ui
+
+namespace side_panel {
+
+// Extracts structured text content from accessibility tree snapshots for
+// BrowserOS LLM features (LLM Chat).
+//
+// Uses depth-first search (DFS) with semantic boundary detection to extract
+// clean, structured text without duplication. Formats output as markdown-like
+// text optimized for LLM consumption.
+//
+// Extraction Strategy:
+//   - Navigation/Banner: Extracted with spacing to separate from content
+//   - Headings: Formatted as markdown (# ## ### etc.)
+//   - Links: Text extracted only (URLs skipped to avoid clutter)
+//   - Images: Alt text extracted as [Image: description]
+//   - Lists: Formatted with proper indentation using tabs
+//   - List items: Indented based on nesting depth
+//   - Paragraphs: Separated with double newlines
+//   - Text nodes: Extracted with appropriate spacing
+//
+// Semantic Boundary Detection:
+//   The extractor stops recursion at headings, links, and images to prevent
+//   extracting their child text multiple times, which would otherwise appear
+//   as duplicates in the output.
+//
+// Thread Safety:
+//   All methods are static and stateless. Safe to call from any thread.
+//
+// Example usage:
+//   active_contents->RequestAXTreeSnapshot(
+//       base::BindOnce([](ui::AXTreeUpdate& update) {
+//         std::u16string text = side_panel::BrowserOSSimplePageExtractor
+//             ::ExtractStructuredText(update);
+//         // Use extracted text...
+//       }), ...);
+//
+class BrowserOSSimplePageExtractor {
+ public:
+  // Extracts structured text from an accessibility tree update.
+  //
+  // Args:
+  //   update: The accessibility tree snapshot from RequestAXTreeSnapshot()
+  //
+  // Returns:
+  //   Structured text with markdown-like formatting, or empty string if:
+  //   - The tree is empty (update.nodes.empty())
+  //   - The tree contains no readable text content
+  static std::u16string ExtractStructuredText(const ui::AXTreeUpdate& update);
+
+  // Utility class - no instances allowed
+  BrowserOSSimplePageExtractor() = delete;
+  ~BrowserOSSimplePageExtractor() = delete;
+  BrowserOSSimplePageExtractor(const BrowserOSSimplePageExtractor&) = delete;
+  BrowserOSSimplePageExtractor& operator=(const BrowserOSSimplePageExtractor&) = delete;
+};
+
+}  // namespace side_panel
+
+#endif  // CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_BROWSEROS_SIMPLE_PAGE_EXTRACTOR_H_
\ No newline at end of file
