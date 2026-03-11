diff --git a/chrome/browser/ui/views/side_panel/side_panel_coordinator.cc b/chrome/browser/ui/views/side_panel/side_panel_coordinator.cc
index 91a6bf5572d1e..e05c4f9679990 100644
--- a/chrome/browser/ui/views/side_panel/side_panel_coordinator.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel_coordinator.cc
@@ -356,9 +356,8 @@ void SidePanelCoordinator::PopulateSidePanel(
   entry->OnEntryShown();
   if (previous_entry) {
     previous_entry->OnEntryHidden();
-  } else {
-    content->RequestFocus();
   }
+  content->RequestFocus();
 
   side_panel->UpdateWidthOnEntryChanged();
 
