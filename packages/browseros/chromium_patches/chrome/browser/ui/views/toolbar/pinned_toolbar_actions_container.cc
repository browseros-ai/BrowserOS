diff --git a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
index 9f47ccc8056035587393766d4032746f0bf1e4f8..b963e60930b5375b2e89abfdcc78b8372f76e1f7 100644
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
@@ -174,6 +175,9 @@ PinnedToolbarActionsContainer::PinnedToolbarActionsContainer(
   // Initialize the pinned action buttons.
   action_view_controller_ = std::make_unique<views::ActionViewController>();
   model_->MaybeMigrateExistingPinnedStates();
+
+  // Ensure actions that should always be pinned are pinned.
+  model_->EnsureAlwaysPinnedActions();
   UpdateViews();
 }
 
@@ -303,6 +307,16 @@ void PinnedToolbarActionsContainer::UpdateAllIcons() {
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
@@ -418,6 +432,10 @@ void PinnedToolbarActionsContainer::OnActionsChanged() {
   drop_weak_ptr_factory_.InvalidateWeakPtrs();
 }
 
+void PinnedToolbarActionsContainer::OnLabelsVisibilityChanged() {
+  UpdateAllLabels();
+}
+
 void PinnedToolbarActionsContainer::WriteDragDataForView(
     View* sender,
     const gfx::Point& press_pt,
@@ -876,6 +894,14 @@ PinnedToolbarActionsContainer::CreateOrGetButtonForAction(
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
