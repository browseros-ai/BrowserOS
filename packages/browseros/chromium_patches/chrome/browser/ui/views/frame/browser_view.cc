diff --git a/chrome/browser/ui/views/frame/browser_view.cc b/chrome/browser/ui/views/frame/browser_view.cc
index a29e3f73ef39b..f274ce83a1160 100644
--- a/chrome/browser/ui/views/frame/browser_view.cc
+++ b/chrome/browser/ui/views/frame/browser_view.cc
@@ -3595,10 +3595,11 @@ void BrowserView::OnSplitTabChanged(const SplitTabChange& change) {
           browser_->tab_strip_model()->GetActiveTab();
 
       if (active_tab->GetSplit() == change.split_id) {
-        if (change.GetVisualsChange()->new_visual_data().split_ratio() !=
-            change.GetVisualsChange()->old_visual_data().split_ratio()) {
-          multi_contents_view_->UpdateSplitRatio(
-              change.GetVisualsChange()->new_visual_data().split_ratio());
+        if (change.GetVisualsChange()->new_visual_data() !=
+            change.GetVisualsChange()->old_visual_data()) {
+          multi_contents_view_->UpdateSplitVisualData(
+              change.GetVisualsChange()->new_visual_data().split_ratio(),
+              change.GetVisualsChange()->new_visual_data().split_layout());
         }
       }
       break;
@@ -4437,8 +4438,9 @@ void BrowserView::ShowSplitView(bool focus_active_view) {
   const int relative_active_position = active_index - first_split_tab_index;
   multi_contents_view_->SetActiveIndex(relative_active_position);
 
-  multi_contents_view_->UpdateSplitRatio(
-      split_data->visual_data()->split_ratio());
+  multi_contents_view_->UpdateSplitVisualData(
+      split_data->visual_data()->split_ratio(),
+      split_data->visual_data()->split_layout());
 
   // Set focus to the active contents avoid reentrency when setting the web
   // contents within MultiContentsView. See crbug.com/458189541 and
@@ -5503,7 +5505,8 @@ bool BrowserView::MaybeUpdateSplitView(content::WebContents* contents) {
     split_tabs::SplitTabData* split_data =
         browser_->tab_strip_model()->GetSplitData(new_tab->GetSplit().value());
     multi_contents_view_->ShowSplitView(
-        split_data->visual_data()->split_ratio());
+        split_data->visual_data()->split_ratio(),
+        split_data->visual_data()->split_layout());
   } else if (current_state != updated_state) {
     multi_contents_view_->CloseSplitView();
   } else {
