diff --git a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.cc b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.cc
index f8298702050da..a666d65dd226c 100644
--- a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.cc
+++ b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.cc
@@ -8,6 +8,11 @@
 #include <type_traits>
 
 #include "base/auto_reset.h"
+#include "chrome/browser/ui/actions/browseros_action_utils.h"
+#include "chrome/browser/ui/actions/chrome_action_id.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_entry.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_entry_id.h"
+#include "chrome/common/extensions/extension_constants.h"
 #include "base/metrics/user_metrics.h"
 #include "base/strings/strcat.h"
 #include "chrome/app/vector_icons/vector_icons.h"
@@ -30,6 +35,7 @@
 #include "chrome/browser/ui/views/toolbar/toolbar_view.h"
 #include "chrome/browser/ui/web_applications/app_browser_controller.h"
 #include "chrome/grit/generated_resources.h"
+#include "third_party/skia/include/core/SkColor.h"
 #include "ui/actions/action_id.h"
 #include "ui/actions/action_utils.h"
 #include "ui/actions/actions.h"
@@ -43,6 +49,8 @@
 #include "ui/views/controls/button/button_controller.h"
 #include "ui/views/view_class_properties.h"
 #include "ui/views/view_utils.h"
+#include "components/prefs/pref_service.h"
+#include "chrome/common/pref_names.h"
 
 namespace {
 // Width of the status indicator shown across the button.
@@ -85,6 +93,30 @@ PinnedActionToolbarButton::PinnedActionToolbarButton(
   GetViewAccessibility().SetDescription(
       std::u16string(), ax::mojom::DescriptionFrom::kAttributeExplicitlyEmpty);
 
+  // Set text from action item if available for BrowserOS actions
+  if (auto* action_item = container_->GetActionItemFor(action_id)) {
+    if (browseros::IsBrowserOSAction(action_id)) {
+      // Check if labels should be shown
+      bool show_labels = true;
+      if (browser_ && browser_->profile()) {
+        show_labels = browser_->profile()->GetPrefs()->GetBoolean(
+            prefs::kBrowserOSShowToolbarLabels);
+      }
+      else {
+      }
+      
+      if (show_labels) {
+        // Use LabelButton::SetText directly to set permanent text
+        views::LabelButton::SetText(action_item->GetText());
+        // Ensure the text is visible
+        SetTextSubpixelRenderingEnabled(false);
+      } else {
+        // Clear the text if labels are disabled
+        views::LabelButton::SetText(std::u16string());
+      }
+    }
+  }
+
   // Normally, the notify action is determined by whether a view is draggable
   // (and is set to press for non-draggable and release for draggable views).
   // However, PinnedActionToolbarButton may be draggable or non-draggable
@@ -234,7 +266,13 @@ void PinnedActionToolbarButton::UpdateIcon() {
                                     ? icons->touch_icon
                                     : icons->icon;
 
-  if (is_icon_visible_ && action_engaged_) {
+  // Special case for Clash of GPTs and Third Party LLM - use custom orange color
+  if (action_id_ == kActionSidePanelShowClashOfGpts ||
+      action_id_ == kActionSidePanelShowThirdPartyLlm) {
+    const SkColor orange = SkColorSetRGB(0xFB, 0x65, 0x18);
+    UpdateIconsWithColors(icon, orange, orange, orange, 
+                          GetForegroundColor(ButtonState::STATE_DISABLED));
+  } else if (is_icon_visible_ && action_engaged_) {
     UpdateIconsWithColors(
         icon, GetColorProvider()->GetColor(kColorToolbarActionItemEngaged),
         GetColorProvider()->GetColor(kColorToolbarActionItemEngaged),
@@ -336,6 +374,26 @@ void PinnedActionToolbarButtonActionViewInterface::ActionItemChangedImpl(
     }
   }
 
+  // Update the text from the action item for BrowserOS actions
+  if (browseros::IsBrowserOSAction(action_view_->GetActionId())) {
+    // Check if labels should be shown
+    bool show_labels = true;
+    if (action_view_->GetBrowser() && action_view_->GetBrowser()->profile()) {
+      show_labels = action_view_->GetBrowser()->profile()->GetPrefs()->GetBoolean(
+          prefs::kBrowserOSShowToolbarLabels);
+    }
+    
+    if (show_labels) {
+      // Use LabelButton::SetText directly to set permanent text
+      action_view_->views::LabelButton::SetText(action_item->GetText());
+      // Ensure the text is visible
+      action_view_->SetTextSubpixelRenderingEnabled(false);
+    } else {
+      // Clear the text if labels are disabled
+      action_view_->views::LabelButton::SetText(std::u16string());
+    }
+  }
+
   // Update whether the action is engaged before updating the view.
   action_view_->SetActionEngaged(
       action_item->GetProperty(kActionItemUnderlineIndicatorKey));
