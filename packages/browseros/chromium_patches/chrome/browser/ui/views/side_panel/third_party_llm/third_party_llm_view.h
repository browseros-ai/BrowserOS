diff --git a/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_view.h b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_view.h
new file mode 100644
index 0000000000000..c6f5e2a95e806
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_view.h
@@ -0,0 +1,32 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_VIEW_H_
+#define CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_VIEW_H_
+
+#include "base/memory/raw_ptr.h"
+#include "ui/views/view.h"
+
+namespace views {
+class WebView;
+}  // namespace views
+
+// Custom view for the Third Party LLM panel that ensures proper cleanup
+// of WebContents during browser shutdown.
+class ThirdPartyLlmView : public views::View {
+ public:
+  ThirdPartyLlmView();
+  ThirdPartyLlmView(const ThirdPartyLlmView&) = delete;
+  ThirdPartyLlmView& operator=(const ThirdPartyLlmView&) = delete;
+  ~ThirdPartyLlmView() override;
+
+  void SetWebView(views::WebView* web_view) { web_view_ = web_view; }
+
+ private:
+  // The WebView that contains our WebContents. We need to track this
+  // to ensure proper cleanup during shutdown.
+  raw_ptr<views::WebView> web_view_ = nullptr;
+};
+
+#endif  // CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_VIEW_H_
\ No newline at end of file
