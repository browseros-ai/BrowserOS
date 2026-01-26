diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.cc b/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.cc
new file mode 100644
index 0000000000000..d4e5f6a7b8c9d
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.cc
@@ -0,0 +1,198 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_suggestion_infobar_delegate.h"
+
+#include <memory>
+#include <utility>
+
+#include "base/strings/string_util.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/infobars/confirm_infobar_creator.h"
+#include "chrome/grit/generated_resources.h"
+#include "components/infobars/content/content_infobar_manager.h"
+#include "components/infobars/core/infobar.h"
+#include "components/vector_icons/vector_icons.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/base/l10n/l10n_util.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+// Generate human-readable action type string
+std::string ActionTypeToString(ActionType type) {
+  switch (type) {
+    case ActionType::kNavigate:
+      return "navigate to";
+    case ActionType::kClick:
+      return "click";
+    case ActionType::kType:
+      return "type in";
+    case ActionType::kScroll:
+      return "scroll";
+    case ActionType::kSelect:
+      return "select";
+    case ActionType::kHover:
+      return "hover over";
+    case ActionType::kKeypress:
+      return "press key";
+    case ActionType::kDragDrop:
+      return "drag and drop";
+    case ActionType::kUpload:
+      return "upload file";
+    case ActionType::kDownload:
+      return "download";
+    case ActionType::kCopy:
+      return "copy";
+    case ActionType::kPaste:
+      return "paste";
+    case ActionType::kFormSubmit:
+      return "submit form";
+    case ActionType::kBack:
+      return "go back";
+    case ActionType::kForward:
+      return "go forward";
+    case ActionType::kRefresh:
+      return "refresh";
+    case ActionType::kNewTab:
+      return "open new tab";
+    case ActionType::kCloseTab:
+      return "close tab";
+  }
+  return "action";
+}
+
+}  // namespace
+
+// static
+void GhostModeSuggestionInfoBarDelegate::Create(
+    content::WebContents* web_contents,
+    const ActionSequence& pattern,
+    AcceptCallback accept_callback,
+    DismissCallback dismiss_callback) {
+  infobars::ContentInfoBarManager* infobar_manager =
+      infobars::ContentInfoBarManager::FromWebContents(web_contents);
+  if (!infobar_manager) {
+    return;
+  }
+  
+  infobar_manager->AddInfoBar(
+      CreateConfirmInfoBar(std::unique_ptr<ConfirmInfoBarDelegate>(
+          new GhostModeSuggestionInfoBarDelegate(
+              pattern, std::move(accept_callback),
+              std::move(dismiss_callback)))));
+}
+
+GhostModeSuggestionInfoBarDelegate::GhostModeSuggestionInfoBarDelegate(
+    const ActionSequence& pattern,
+    AcceptCallback accept_callback,
+    DismissCallback dismiss_callback)
+    : pattern_(pattern),
+      accept_callback_(std::move(accept_callback)),
+      dismiss_callback_(std::move(dismiss_callback)) {}
+
+GhostModeSuggestionInfoBarDelegate::~GhostModeSuggestionInfoBarDelegate() =
+    default;
+
+infobars::InfoBarDelegate::InfoBarIdentifier
+GhostModeSuggestionInfoBarDelegate::GetIdentifier() const {
+  // Using identifier 131 (after BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 130)
+  return GHOST_MODE_SUGGESTION_INFOBAR_DELEGATE;
+}
+
+const gfx::VectorIcon&
+GhostModeSuggestionInfoBarDelegate::GetVectorIcon() const {
+  return vector_icons::kSmartDisplayIcon;
+}
+
+std::u16string GhostModeSuggestionInfoBarDelegate::GetMessageText() const {
+  // Build message with pattern info
+  std::string message = "👻 Ghost Mode: Detected a repeated pattern \"";
+  message += pattern_.name;
+  message += "\" (" + std::to_string(pattern_.actions.size()) + " steps, ";
+  message += std::to_string(pattern_.occurrence_count) + " times). ";
+  message += "Convert to workflow?";
+  
+  return base::UTF8ToUTF16(message);
+}
+
+int GhostModeSuggestionInfoBarDelegate::GetButtons() const {
+  return BUTTON_OK | BUTTON_CANCEL;
+}
+
+std::u16string GhostModeSuggestionInfoBarDelegate::GetButtonLabel(
+    InfoBarButton button) const {
+  if (button == BUTTON_OK) {
+    return u"Create Workflow";
+  }
+  return u"Don't Ask Again";
+}
+
+bool GhostModeSuggestionInfoBarDelegate::Accept() {
+  if (accept_callback_) {
+    std::move(accept_callback_).Run(pattern_);
+  }
+  return true;  // Close infobar
+}
+
+bool GhostModeSuggestionInfoBarDelegate::Cancel() {
+  if (dismiss_callback_) {
+    std::move(dismiss_callback_).Run(pattern_.id);
+  }
+  return true;  // Close infobar
+}
+
+void GhostModeSuggestionInfoBarDelegate::InfoBarDismissed() {
+  // User clicked X - treat as "ask later"
+  // Don't call dismiss callback, just let it close
+}
+
+bool GhostModeSuggestionInfoBarDelegate::IsCloseable() const {
+  return true;  // Allow closing with X button
+}
+
+// GhostModeSuggestionInfoBar static methods
+
+// static
+GhostModeSuggestionInfoBar::Content
+GhostModeSuggestionInfoBar::CreateContent(const ActionSequence& pattern) {
+  Content content;
+  content.title = "Repetitive pattern detected";
+  content.description = FormatPatternDescription(pattern);
+  content.pattern_id = pattern.id;
+  content.step_count = static_cast<int>(pattern.actions.size());
+  content.confidence = pattern.confidence_score;
+  
+  // Extract unique URLs from actions
+  std::set<std::string> urls;
+  for (const auto& action : pattern.actions) {
+    if (action.url.is_valid()) {
+      urls.insert(action.url.host());
+    }
+  }
+  content.sample_urls = std::vector<std::string>(urls.begin(), urls.end());
+  
+  return content;
+}
+
+// static
+std::string GhostModeSuggestionInfoBar::FormatPatternDescription(
+    const ActionSequence& pattern) {
+  std::vector<std::string> steps;
+  for (const auto& action : pattern.actions) {
+    std::string step = ActionTypeToString(action.type);
+    if (action.url.is_valid()) {
+      step += " " + action.url.host();
+    }
+    steps.push_back(step);
+  }
+  return base::JoinString(steps, " → ");
+}
+
+// static
+std::string GhostModeSuggestionInfoBar::GetConfidenceLevel(double score) {
+  if (score >= 0.9) return "High";
+  if (score >= 0.7) return "Medium";
+  return "Low";
+}
+
+}  // namespace browseros::ghost_mode
