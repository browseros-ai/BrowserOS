diff --git a/chrome/browser/ui/views/frame/browser_view_layout.h b/chrome/browser/ui/views/frame/browser_view_layout.h
index a4602bd1f5d01f1cab888bc5e045d1b8ad394c6a..9906200aa517a411ab24b1bdf4a3099eee8348b5 100644
--- a/chrome/browser/ui/views/frame/browser_view_layout.h
+++ b/chrome/browser/ui/views/frame/browser_view_layout.h
@@ -15,6 +15,7 @@
 #include "ui/views/layout/layout_manager.h"
 
 class BookmarkBarView;
+class AgentPaneView;
 class BrowserView;
 class BrowserViewLayoutDelegate;
 class ImmersiveModeController;
@@ -63,7 +64,8 @@ class BrowserViewLayout : public views::LayoutManager {
                     views::View* right_aligned_side_panel_separator,
                     views::View* side_panel_rounded_corner,
                     ImmersiveModeController* immersive_mode_controller,
-                    views::View* contents_separator);
+                    views::View* contents_separator,
+                    AgentPaneView* agent_pane);
 
   BrowserViewLayout(const BrowserViewLayout&) = delete;
   BrowserViewLayout& operator=(const BrowserViewLayout&) = delete;
@@ -133,6 +135,10 @@ class BrowserViewLayout : public views::LayoutManager {
   int LayoutBookmarkBar(int top);
   int LayoutInfoBar(int top);
 
+  // Returns the horizontal origin for a view in its current parent's
+  // coordinate space. Some top-chrome children reparent at runtime.
+  int LeftEdgeFor(const views::View* view) const;
+
   // Layout the |contents_container_| view between the coordinates |top| and
   // |bottom|. See browser_view.h for details of the relationship between
   // |contents_container_| and other views.
@@ -183,6 +189,7 @@ class BrowserViewLayout : public views::LayoutManager {
   const raw_ptr<views::View> side_panel_rounded_corner_;
   const raw_ptr<ImmersiveModeController> immersive_mode_controller_;
   const raw_ptr<views::View> contents_separator_;
+  const raw_ptr<AgentPaneView> agent_pane_;
 
   // These views are dynamically set.
   raw_ptr<views::View> webui_tab_strip_ = nullptr;
@@ -202,6 +209,7 @@ class BrowserViewLayout : public views::LayoutManager {
   // BrowserView.
   // TODO(jamescook): Remove this and just use browser_view_->GetLocalBounds().
   gfx::Rect vertical_layout_rect_;
+  int agent_pane_width_ = 0;
 
   // The host for use in positioning the web contents modal dialog.
   std::unique_ptr<WebContentsModalDialogHostViews> dialog_host_;
