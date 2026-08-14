diff --git a/chrome/browser/ui/views/frame/browser_view_layout.cc b/chrome/browser/ui/views/frame/browser_view_layout.cc
index 39fc0f21ff96ce739550aef86c8b3ab4bd7bf3d2..41042dc267ab2851472b33a9121c15a0ce876af4 100644
--- a/chrome/browser/ui/views/frame/browser_view_layout.cc
+++ b/chrome/browser/ui/views/frame/browser_view_layout.cc
@@ -25,6 +25,7 @@
 #include "chrome/browser/ui/views/bookmarks/bookmark_bar_view.h"
 #include "chrome/browser/ui/views/download/download_shelf_view.h"
 #include "chrome/browser/ui/views/exclusive_access_bubble_views.h"
+#include "chrome/browser/ui/views/frame/agent_pane_view.h"
 #include "chrome/browser/ui/views/frame/browser_non_client_frame_view.h"
 #include "chrome/browser/ui/views/frame/browser_view_layout_delegate.h"
 #include "chrome/browser/ui/views/frame/contents_layout_manager.h"
@@ -63,6 +64,10 @@ namespace {
 // of the omnibox.
 const int kConstrainedWindowOverlap = 3;
 
+// The browser column remains large enough to keep its top chrome and contents
+// usable while the left agent pane is resized.
+constexpr int kAgentPaneMinBrowserWidth = 400;
+
 // The normal clipping created by `View::Paint()` may not cover the bottom of
 // the TopContainerView at certain scale factor because both of the position and
 // the height might be roudned down. This function sets the clip path that
@@ -204,7 +209,8 @@ BrowserViewLayout::BrowserViewLayout(
     views::View* right_aligned_side_panel_separator,
     views::View* side_panel_rounded_corner,
     ImmersiveModeController* immersive_mode_controller,
-    views::View* contents_separator)
+    views::View* contents_separator,
+    AgentPaneView* agent_pane)
     : delegate_(std::move(delegate)),
       browser_view_(browser_view),
       window_scrim_(window_scrim),
@@ -221,6 +227,7 @@ BrowserViewLayout::BrowserViewLayout(
       side_panel_rounded_corner_(side_panel_rounded_corner),
       immersive_mode_controller_(immersive_mode_controller),
       contents_separator_(contents_separator),
+      agent_pane_(agent_pane),
       tab_strip_(tab_strip),
       dialog_host_(std::make_unique<WebContentsModalDialogHostViews>(this)) {}
 
@@ -284,10 +291,14 @@ gfx::Size BrowserViewLayout::GetMinimumSize(const views::View* host) const {
       toolbar_size.height() + bookmark_bar_size.height() +
       infobar_container_size.height() + contents_size.height();
 
-  const int min_width = std::max(
+  int min_width = std::max(
       {tabstrip_size.width(), toolbar_size.width(), bookmark_bar_size.width(),
        infobar_container_size.width(), contents_size.width()});
 
+  if (delegate_->GetAgentPaneWidth() > 0 && !delegate_->IsAgentPaneOnly()) {
+    min_width += AgentPaneView::kMinimumWidth + AgentPaneView::kResizeAreaWidth;
+  }
+
   return gfx::Size(min_width, min_height);
 }
 
@@ -308,6 +319,27 @@ void BrowserViewLayout::Layout(views::View* browser_view) {
     window_scrim_->SetBoundsRect(vertical_layout_rect_);
   }
 
+  agent_pane_width_ = 0;
+  if (agent_pane_) {
+    const int total_width = vertical_layout_rect_.width();
+    const int requested_width = delegate_->GetAgentPaneWidth();
+    if (requested_width > 0 && total_width > 0) {
+      agent_pane_width_ =
+          delegate_->IsAgentPaneOnly()
+              ? total_width
+              : std::clamp(requested_width, AgentPaneView::kMinimumWidth,
+                           std::max(AgentPaneView::kMinimumWidth,
+                                    total_width - kAgentPaneMinBrowserWidth));
+    }
+    SetViewVisibility(agent_pane_, agent_pane_width_ > 0);
+    if (agent_pane_width_ > 0) {
+      agent_pane_->SetBounds(vertical_layout_rect_.x(),
+                             vertical_layout_rect_.y(), agent_pane_width_,
+                             vertical_layout_rect_.height());
+      vertical_layout_rect_.Inset(gfx::Insets().set_left(agent_pane_width_));
+    }
+  }
+
   int top_inset = delegate_->GetTopInsetInBrowserView();
   int top = LayoutTitleBarForWebApp(top_inset);
   if (delegate_->ShouldLayoutTabStrip()) {
@@ -481,6 +513,13 @@ int BrowserViewLayout::LayoutTabStripRegion(int top) {
   gfx::Rect tab_strip_region_bounds(
       delegate_->GetBoundsForTabStripRegionInBrowserView());
 
+  if (agent_pane_ && agent_pane_->GetVisible()) {
+    // The traffic lights sit over the agent pane. Reclaim their frame-provided
+    // inset and express this child in top_container_ coordinates.
+    tab_strip_region_bounds.set_x(0);
+    tab_strip_region_bounds.set_width(vertical_layout_rect_.width());
+  }
+
   if (web_app_frame_toolbar_) {
     tab_strip_region_bounds.Inset(gfx::Insets::TLBR(
         0, 0, 0, web_app_frame_toolbar_->GetPreferredSize().width()));
@@ -503,7 +542,7 @@ int BrowserViewLayout::LayoutWebUITabStrip(int top) {
     return top;
   }
   webui_tab_strip_->SetBounds(
-      vertical_layout_rect_.x(), top, vertical_layout_rect_.width(),
+      LeftEdgeFor(webui_tab_strip_), top, vertical_layout_rect_.width(),
       webui_tab_strip_->GetHeightForWidth(vertical_layout_rect_.width()));
   return webui_tab_strip_->bounds().bottom();
 }
@@ -514,8 +553,7 @@ int BrowserViewLayout::LayoutToolbar(int top) {
   bool toolbar_visible = delegate_->IsToolbarVisible();
   int height = toolbar_visible ? toolbar_->GetPreferredSize().height() : 0;
   SetViewVisibility(toolbar_, toolbar_visible);
-  toolbar_->SetBounds(vertical_layout_rect_.x(), top, browser_view_width,
-                      height);
+  toolbar_->SetBounds(LeftEdgeFor(toolbar_), top, browser_view_width, height);
   SetClipPathWithBottomAllowance(toolbar_);
   return toolbar_->bounds().bottom();
 }
@@ -533,12 +571,12 @@ int BrowserViewLayout::LayoutBookmarkAndInfoBars(int top, int browser_view_y) {
     SetViewVisibility(contents_separator_, true);
     const int separator_height =
         contents_separator_->GetPreferredSize().height();
-    contents_separator_->SetBounds(vertical_layout_rect_.x(), top,
+    contents_separator_->SetBounds(LeftEdgeFor(contents_separator_), top,
                                    vertical_layout_rect_.width(),
                                    separator_height);
     if (loading_bar_) {
       SetViewVisibility(loading_bar_, true);
-      loading_bar_->SetBounds(vertical_layout_rect_.x(), top - 2,
+      loading_bar_->SetBounds(LeftEdgeFor(loading_bar_), top - 2,
                               vertical_layout_rect_.width(),
                               separator_height + 2);
       top_container_->ReorderChildView(loading_bar_,
@@ -560,13 +598,14 @@ int BrowserViewLayout::LayoutBookmarkBar(int top) {
     SetViewVisibility(bookmark_bar_, false);
     // TODO(jamescook): Don't change the bookmark bar height when it is
     // invisible, so we can use its height for layout even in that state.
-    bookmark_bar_->SetBounds(0, top, browser_view_->width(), 0);
+    bookmark_bar_->SetBounds(LeftEdgeFor(bookmark_bar_), top,
+                             vertical_layout_rect_.width(), 0);
     return top;
   }
 
   bookmark_bar_->SetInfoBarVisible(IsInfobarVisible());
   int bookmark_bar_height = bookmark_bar_->GetPreferredSize().height();
-  bookmark_bar_->SetBounds(vertical_layout_rect_.x(), top,
+  bookmark_bar_->SetBounds(LeftEdgeFor(bookmark_bar_), top,
                            vertical_layout_rect_.width(), bookmark_bar_height);
   SetClipPathWithBottomAllowance(bookmark_bar_);
   if (!ui::IsPixelCanvasRecordingEnabled()) {
@@ -587,6 +626,16 @@ int BrowserViewLayout::LayoutBookmarkBar(int top) {
 }
 
 int BrowserViewLayout::LayoutInfoBar(int top) {
+  if (delegate_->IsAgentPaneOnly()) {
+    // Keep the hidden height current so the first SPLIT layout offsets browser
+    // contents below an existing infobar instead of overlapping it for a frame.
+    infobar_container_->SetBounds(
+        LeftEdgeFor(infobar_container_), top, vertical_layout_rect_.width(),
+        infobar_container_->GetPreferredSize().height());
+    SetViewVisibility(infobar_container_, false);
+    return top;
+  }
+
   // In immersive fullscreen or when top-chrome is fully hidden due to the page
   // gesture scroll slide behavior, the infobar always starts near the top of
   // the screen.
@@ -604,15 +653,38 @@ int BrowserViewLayout::LayoutInfoBar(int top) {
   infobar_top += delegate_->GetExtraInfobarOffset();
   SetViewVisibility(infobar_container_, IsInfobarVisible());
   infobar_container_->SetBounds(
-      vertical_layout_rect_.x(), infobar_top, vertical_layout_rect_.width(),
+      LeftEdgeFor(infobar_container_), infobar_top,
+      vertical_layout_rect_.width(),
       infobar_container_->GetPreferredSize().height());
   return content_top;
 }
 
+int BrowserViewLayout::LeftEdgeFor(const views::View* view) const {
+  return view && view->parent() == top_container_ ? 0
+                                                  : vertical_layout_rect_.x();
+}
+
 void BrowserViewLayout::LayoutContentsContainerView(int top, int bottom) {
   TRACE_EVENT0("ui", "BrowserViewLayout::LayoutContentsContainerView");
   // |contents_container_| contains web page contents and devtools.
   // See browser_view.h for details.
+  if (delegate_->IsAgentPaneOnly()) {
+    SetViewVisibility(contents_container_, false);
+    // Preserve the side-panel coordinator's visibility state so an open panel
+    // returns in SPLIT. Empty bounds keep it out of the agent-only surface.
+    if (unified_side_panel_) {
+      unified_side_panel_->SetBoundsRect(gfx::Rect());
+    }
+    if (left_aligned_side_panel_separator_) {
+      SetViewVisibility(left_aligned_side_panel_separator_, false);
+    }
+    if (right_aligned_side_panel_separator_) {
+      SetViewVisibility(right_aligned_side_panel_separator_, false);
+    }
+    SetViewVisibility(side_panel_rounded_corner_, false);
+    return;
+  }
+  SetViewVisibility(contents_container_, true);
   gfx::Rect contents_container_bounds(vertical_layout_rect_.x(), top,
                                       vertical_layout_rect_.width(),
                                       std::max(0, bottom - top));
@@ -705,7 +777,7 @@ void BrowserViewLayout::LayoutSidePanelView(
     // to the ui direction, move `contents_container_bounds` after the side
     // panel. Also leave space for the separator.
     contents_container_bounds.set_x(
-        side_panel_visible_width +
+        contents_container_bounds.x() + side_panel_visible_width +
         side_panel_separator->GetPreferredSize().width());
     side_panel_bounds.set_x(side_panel_bounds.x() - (side_panel_bounds.width() -
                                                      side_panel_visible_width));
@@ -778,7 +850,8 @@ void BrowserViewLayout::UpdateTopContainerBounds() {
   // layout and we assume that this is the case.
   height = std::max(height, delegate_->GetTopInsetInBrowserView());
 
-  gfx::Rect top_container_bounds(vertical_layout_rect_.width(), height);
+  gfx::Rect top_container_bounds(vertical_layout_rect_.x(), 0,
+                                 vertical_layout_rect_.width(), height);
 
   if (delegate_->IsTopControlsSlideBehaviorEnabled()) {
     // If the top controls are fully hidden, then it's positioned outside the
@@ -799,8 +872,12 @@ void BrowserViewLayout::UpdateTopContainerBounds() {
 int BrowserViewLayout::LayoutDownloadShelf(int bottom) {
   TRACE_EVENT0("ui", "BrowserViewLayout::LayoutDownloadShelf");
   if (download_shelf_ && download_shelf_->GetVisible()) {
+    if (delegate_->IsAgentPaneOnly()) {
+      download_shelf_->SetBoundsRect(gfx::Rect());
+      return bottom;
+    }
     const int height = download_shelf_->GetPreferredSize().height();
-    download_shelf_->SetBounds(vertical_layout_rect_.x(), bottom - height,
+    download_shelf_->SetBounds(LeftEdgeFor(download_shelf_), bottom - height,
                                vertical_layout_rect_.width(), height);
     bottom -= height;
   }
