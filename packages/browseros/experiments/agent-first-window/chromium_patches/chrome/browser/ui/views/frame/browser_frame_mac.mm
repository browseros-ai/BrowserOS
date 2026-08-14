diff --git a/chrome/browser/ui/views/frame/browser_frame_mac.mm b/chrome/browser/ui/views/frame/browser_frame_mac.mm
index 9f478e1478d4128c196ea9eacac00141cd5f110c..2c4ffd4444d154818274a0eec0279100c2bda500 100644
--- a/chrome/browser/ui/views/frame/browser_frame_mac.mm
+++ b/chrome/browser/ui/views/frame/browser_frame_mac.mm
@@ -156,6 +156,11 @@ void BrowserFrameMac::GetWindowFrameTitlebarHeight(
   if (browser_view_ && browser_view_->frame() &&
       browser_view_->frame()->GetFrameView()) {
     *override_titlebar_height = true;
+    if (browser_view_->agent_window_mode() ==
+        BrowserView::AgentWindowMode::kAgentOnly) {
+      *titlebar_height = BrowserView::kAgentTitleBarHeight;
+      return;
+    }
     *titlebar_height =
         browser_view_->GetTabStripHeight() +
         browser_view_->frame()->GetFrameView()->GetTopInset(true);
