diff --git a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
index 695f2d7027ca09fd8153099893ed81dc37914347..5e0877bdfbb054d770ccf27e6087b167cd5b64e0 100644
--- a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
+++ b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
@@ -56,6 +56,9 @@ class PinnedToolbarActionsContainer
   // ToolbarIconContainerView:
   void UpdateAllIcons() override;
 
+  // Updates label visibility on all buttons based on pref.
+  void UpdateAllLabels();
+
   // views::View:
   void AddedToWidget() override;
   bool GetDropFormats(int* formats,
@@ -72,6 +75,7 @@ class PinnedToolbarActionsContainer
   void OnActionAddedLocally(actions::ActionId id) override;
   void OnActionRemovedLocally(actions::ActionId id) override;
   void OnActionsChanged() override;
+  void OnLabelsVisibilityChanged() override;
 
   // views::DragController:
   void WriteDragDataForView(View* sender,
