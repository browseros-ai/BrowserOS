diff --git a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
index 18f3e41abcb7ad379240a6213c201f1dea7f52be..38688a08c79291541fe9933122ccfe972775caee 100644
--- a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
+++ b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
@@ -18,6 +18,7 @@
 #include "base/scoped_observation.h"
 #include "base/task/single_thread_task_runner.h"
 #include "base/time/time.h"
+#include "chrome/browser/browseros/core/browseros_action_utils.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser_actions.h"
@@ -173,6 +174,9 @@ PinnedToolbarActionsContainer::PinnedToolbarActionsContainer(
   // Initialize the pinned action buttons.
   action_view_controller_ = std::make_unique<views::ActionViewController>();
   model_->MaybeMigrateExistingPinnedStates();
+
+  // Ensure actions that should always be pinned are pinned.
+  model_->EnsureAlwaysPinnedActions();
   UpdateViews();
 }
 
@@ -296,6 +300,16 @@ void PinnedToolbarActionsContainer::UpdateAllIcons() {
   }
 }
 
+void PinnedToolbarActionsContainer::UpdateAllLabels() {
+  for (PinnedActionToolbarButton* const pinned_button : pinned_buttons_) {
+    pinned_button->UpdateLabelVisibility();
+  }
+  for (PinnedActionToolbarButton* const popped_out_button :
+       popped_out_buttons_) {
+    popped_out_button->UpdateLabelVisibility();
+  }
+}
+
 void PinnedToolbarActionsContainer::AddedToWidget() {
   OnThemeChanged();
 }
@@ -411,6 +425,10 @@ void PinnedToolbarActionsContainer::OnActionsChanged() {
   drop_weak_ptr_factory_.InvalidateWeakPtrs();
 }
 
+void PinnedToolbarActionsContainer::OnLabelsVisibilityChanged() {
+  UpdateAllLabels();
+}
+
 void PinnedToolbarActionsContainer::WriteDragDataForView(
     View* sender,
     const gfx::Point& press_pt,
@@ -868,6 +886,14 @@ PinnedToolbarActionsContainer::CreateOrGetButtonForAction(
   action_view_controller_->CreateActionViewRelationship(
       button.get(), action_item->GetAsWeakPtr());
 
+  // Set high priority for BrowserOS actions to ensure they're always visible
+  if (browseros::IsBrowserOSAction(id)) {
+    button->SetProperty(
+        kToolbarButtonFlexPriorityKey,
+        static_cast<std::underlying_type_t<PinnedToolbarActionFlexPriority>>(
+            PinnedToolbarActionFlexPriority::kHigh));
+  }
+
   button->SetPaintToLayer();
   button->layer()->SetFillsBoundsOpaquely(false);
   return button;
