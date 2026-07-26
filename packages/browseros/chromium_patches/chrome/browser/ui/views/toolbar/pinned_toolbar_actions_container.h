diff --git a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
index 4b17a33e211a8..7225867cb31d0 100644
--- a/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
+++ b/chrome/browser/ui/views/toolbar/pinned_toolbar_actions_container.h
@@ -58,6 +58,9 @@ class PinnedToolbarActionsContainer
   // ToolbarIconContainerView:
   void UpdateAllIcons() override;
 
+  // Updates label visibility on all buttons based on pref.
+  void UpdateAllLabels();
+
   // views::View:
   void AddedToWidget() override;
   bool GetDropFormats(int* formats,
@@ -74,6 +77,7 @@ class PinnedToolbarActionsContainer
   void OnActionAddedLocally(actions::ActionId id) override;
   void OnActionRemovedLocally(actions::ActionId id) override;
   void OnActionsChanged() override;
+  void OnLabelsVisibilityChanged() override;
 
   // views::DragController:
   void WriteDragDataForView(View* sender,
