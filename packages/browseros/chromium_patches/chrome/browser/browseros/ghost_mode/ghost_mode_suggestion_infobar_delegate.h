diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.h b/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.h
new file mode 100644
index 0000000000000..c3d4e5f6a7b8c
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.h
@@ -0,0 +1,108 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SUGGESTION_INFOBAR_DELEGATE_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SUGGESTION_INFOBAR_DELEGATE_H_
+
+#include <string>
+
+#include "base/functional/callback.h"
+#include "base/memory/raw_ptr.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "components/infobars/core/confirm_infobar_delegate.h"
+
+class Profile;
+
+namespace content {
+class WebContents;
+}
+
+namespace browseros::ghost_mode {
+
+// InfoBar delegate for Ghost Mode pattern suggestions.
+// Shows when a repetitive pattern is detected and offers to:
+// 1. Convert to workflow (Accept)
+// 2. Dismiss forever (Cancel)
+// 3. Ask later (Close)
+class GhostModeSuggestionInfoBarDelegate : public ConfirmInfoBarDelegate {
+ public:
+  // Callback types for user actions
+  using AcceptCallback = base::OnceCallback<void(const ActionSequence&)>;
+  using DismissCallback = base::OnceCallback<void(const std::string&)>;
+  
+  // Create and show the infobar
+  static void Create(content::WebContents* web_contents,
+                     const ActionSequence& pattern,
+                     AcceptCallback accept_callback,
+                     DismissCallback dismiss_callback);
+  
+  GhostModeSuggestionInfoBarDelegate(const GhostModeSuggestionInfoBarDelegate&) = delete;
+  GhostModeSuggestionInfoBarDelegate& operator=(const GhostModeSuggestionInfoBarDelegate&) = delete;
+  ~GhostModeSuggestionInfoBarDelegate() override;
+
+ private:
+  GhostModeSuggestionInfoBarDelegate(const ActionSequence& pattern,
+                                      AcceptCallback accept_callback,
+                                      DismissCallback dismiss_callback);
+
+  // ConfirmInfoBarDelegate:
+  infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
+  const gfx::VectorIcon& GetVectorIcon() const override;
+  std::u16string GetMessageText() const override;
+  int GetButtons() const override;
+  std::u16string GetButtonLabel(InfoBarButton button) const override;
+  bool Accept() override;
+  bool Cancel() override;
+  void InfoBarDismissed() override;
+  bool IsCloseable() const override;
+  
+  // Pattern that triggered this suggestion
+  ActionSequence pattern_;
+  
+  // Callbacks for user actions
+  AcceptCallback accept_callback_;
+  DismissCallback dismiss_callback_;
+};
+
+// Custom InfoBar UI for Ghost Mode suggestions with richer display
+// Shows pattern preview, confidence indicator, and action buttons
+class GhostModeSuggestionInfoBar {
+ public:
+  // InfoBar content structure
+  struct Content {
+    std::string title;
+    std::string description;
+    std::string pattern_id;
+    int step_count;
+    double confidence;
+    std::vector<std::string> sample_urls;
+  };
+  
+  // Create InfoBar content from pattern
+  static Content CreateContent(const ActionSequence& pattern);
+  
+  // Format pattern as human-readable description
+  static std::string FormatPatternDescription(const ActionSequence& pattern);
+  
+  // Get confidence level text (High/Medium/Low)
+  static std::string GetConfidenceLevel(double score);
+};
+
+// Observer for InfoBar interactions
+class GhostModeSuggestionObserver {
+ public:
+  virtual ~GhostModeSuggestionObserver() = default;
+  
+  // Called when user accepts the suggestion
+  virtual void OnSuggestionAccepted(const ActionSequence& pattern) = 0;
+  
+  // Called when user dismisses the suggestion permanently
+  virtual void OnSuggestionDismissed(const std::string& pattern_id) = 0;
+  
+  // Called when user asks to be reminded later
+  virtual void OnSuggestionDeferred(const std::string& pattern_id) = 0;
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SUGGESTION_INFOBAR_DELEGATE_H_
