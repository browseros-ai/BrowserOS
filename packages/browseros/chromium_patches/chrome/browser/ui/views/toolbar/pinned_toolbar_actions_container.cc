diff --git a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
index 4a8e5f200dbc5..3efe127a138b4 100644
--- a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
+++ b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.cc
@@ -19,6 +19,8 @@
 #include "base/task/single_thread_task_runner.h"
 #include "base/time/time.h"
 #include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/actions/browseros_actions_config.h"
+#include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser_actions.h"
 #include "chrome/browser/ui/browser_element_identifiers.h"
 #include "chrome/browser/ui/layout_constants.h"
@@ -146,6 +148,9 @@ PinnedToolbarActionsContainer::PinnedToolbarActionsContainer(
   // Initialize the pinned action buttons.
   action_view_controller_ = std::make_unique<views::ActionViewController>();
   model_->MaybeMigrateExistingPinnedStates();
+
+  // Ensure actions that should always be pinned are pinned.
+  model_->EnsureAlwaysPinnedActions();
   UpdateViews();
 }
 
@@ -822,6 +827,14 @@ PinnedToolbarActionsContainer::CreateOrGetButtonForAction(
   action_view_controller_->CreateActionViewRelationship(
       button.get(), GetActionItemFor(id)->GetAsWeakPtr());
 
+  // Set high priority for BlockBrowser actions to ensure they're always visible
+  if (browseros::IsBlockBrowserAction(id)) {
+    button->SetProperty(
+        kToolbarButtonFlexPriorityKey,
+        static_cast<std::underlying_type_t<PinnedToolbarActionFlexPriority>>(
+            PinnedToolbarActionFlexPriority::kHigh));
+  }
+
   button->SetPaintToLayer();
   button->layer()->SetFillsBoundsOpaquely(false);
   return button;
