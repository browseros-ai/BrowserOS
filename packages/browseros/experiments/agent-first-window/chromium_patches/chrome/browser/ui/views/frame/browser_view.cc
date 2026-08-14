diff --git a/chrome/browser/ui/views/frame/browser_view.cc b/chrome/browser/ui/views/frame/browser_view.cc
index 3c0e53b5ade77b589f3899064aeee5bb67e3871f..91a24aeef71a5bf5b116a27599f76da1ffd1e17f 100644
--- a/chrome/browser/ui/views/frame/browser_view.cc
+++ b/chrome/browser/ui/views/frame/browser_view.cc
@@ -118,6 +118,7 @@
 #include "chrome/browser/ui/views/extensions/extensions_toolbar_container.h"
 #include "chrome/browser/ui/views/eye_dropper/eye_dropper.h"
 #include "chrome/browser/ui/views/find_bar_host.h"
+#include "chrome/browser/ui/views/frame/agent_pane_view.h"
 #include "chrome/browser/ui/views/frame/app_menu_button.h"
 #include "chrome/browser/ui/views/frame/browser_frame.h"
 #include "chrome/browser/ui/views/frame/browser_view_layout.h"
@@ -817,8 +818,27 @@ class BrowserViewLayoutDelegateImpl : public BrowserViewLayoutDelegate {
     return browser_view_->IsToolbarVisible();
   }
 
+  int GetAgentPaneWidth() const override {
+    switch (browser_view_->agent_window_mode()) {
+      case BrowserView::AgentWindowMode::kDisabled:
+        return 0;
+      case BrowserView::AgentWindowMode::kAgentOnly:
+        return browser_view_->width();
+      case BrowserView::AgentWindowMode::kSplit: {
+        const int stored_width = browser_view_->agent_pane()->split_width();
+        return stored_width > 0 ? stored_width : browser_view_->width() * 2 / 5;
+      }
+    }
+    NOTREACHED();
+  }
+
+  bool IsAgentPaneOnly() const override {
+    return browser_view_->agent_window_mode() ==
+           BrowserView::AgentWindowMode::kAgentOnly;
+  }
+
   bool IsBookmarkBarVisible() const override {
-    return browser_view_->IsBookmarkBarVisible();
+    return !IsAgentPaneOnly() && browser_view_->IsBookmarkBarVisible();
   }
 
   bool IsContentsSeparatorEnabled() const override {
@@ -1113,6 +1133,11 @@ BrowserView::BrowserView(std::unique_ptr<Browser> browser)
   side_panel_rounded_corner_ =
       AddChildView(std::make_unique<SidePanelRoundedCorner>(this));
 
+  if (GetIsNormalType()) {
+    agent_pane_ = AddChildView(std::make_unique<AgentPaneView>(this));
+    agent_mode_ = AgentWindowMode::kAgentOnly;
+  }
+
   // InfoBarContainer needs to be added as a child here for drop-shadow, but
   // needs to come after toolbar in focus order (see EnsureFocusOrder()).
   infobar_container_ =
@@ -1229,6 +1254,7 @@ BrowserView::~BrowserView() {
   right_aligned_side_panel_separator_ = nullptr;
   left_aligned_side_panel_separator_ = nullptr;
   side_panel_rounded_corner_ = nullptr;
+  agent_pane_ = nullptr;
   toolbar_button_provider_ = nullptr;
 
   // Child views maintain PrefMember attributes that point to
@@ -1283,6 +1309,45 @@ BrowserView* BrowserView::GetBrowserViewForBrowser(const Browser* browser) {
   return GetBrowserViewForNativeWindow(browser->window()->GetNativeWindow());
 }
 
+void BrowserView::SetAgentWindowMode(AgentWindowMode mode) {
+  if (!agent_pane_ || agent_mode_ == mode) {
+    return;
+  }
+
+  agent_mode_ = mode;
+  agent_pane_->EnsureContents();
+  agent_pane_->SetShowResizeArea(mode == AgentWindowMode::kSplit);
+  InvalidateLayout();
+  if (frame_ && frame_->GetFrameView()) {
+    frame_->GetFrameView()->UpdateMinimumSize();
+  }
+  DeprecatedLayoutImmediately();
+  if (mode == AgentWindowMode::kAgentOnly) {
+    agent_pane_->RestoreFocus();
+  } else {
+    RestoreFocus();
+  }
+}
+
+void BrowserView::ToggleAgentSplit() {
+  if (agent_mode_ == AgentWindowMode::kSplit && GetActiveWebContents()) {
+    if ((browser_->HasFindBarController() &&
+         browser_->GetFindBarController()->find_bar()->IsFindBarVisible()) ||
+        IsDownloadShelfVisible()) {
+      return;
+    }
+    auto* modal_manager =
+        web_modal::WebContentsModalDialogManager::FromWebContents(
+            GetActiveWebContents());
+    if (modal_manager && modal_manager->IsDialogActive()) {
+      return;
+    }
+  }
+  SetAgentWindowMode(agent_mode_ == AgentWindowMode::kSplit
+                         ? AgentWindowMode::kAgentOnly
+                         : AgentWindowMode::kSplit);
+}
+
 void BrowserView::SetDownloadShelfForTest(DownloadShelf* download_shelf) {
   download_shelf_ = download_shelf;
 }
@@ -1326,6 +1391,11 @@ gfx::Size BrowserView::GetWebAppFrameToolbarPreferredSize() const {
 
 #if BUILDFLAG(IS_MAC)
 bool BrowserView::UsesImmersiveFullscreenMode() const {
+  // Milestone 1 does not support macOS immersive mode because it reparents
+  // top chrome into an overlay widget that spans the agent pane.
+  if (GetIsNormalType()) {
+    return false;
+  }
   const bool is_pwa =
       base::FeatureList::IsEnabled(features::kImmersiveFullscreenPWAs) &&
       GetIsWebAppType();
@@ -1335,6 +1405,9 @@ bool BrowserView::UsesImmersiveFullscreenMode() const {
 }
 
 bool BrowserView::UsesImmersiveFullscreenTabbedMode() const {
+  if (GetIsNormalType()) {
+    return false;
+  }
   return (GetSupportsTabStrip() &&
           base::FeatureList::IsEnabled(features::kImmersiveFullscreen) &&
           base::FeatureList::IsEnabled(features::kImmersiveFullscreenTabs)) &&
@@ -1360,6 +1433,10 @@ bool BrowserView::GetTabStripVisible() const {
 }
 
 bool BrowserView::ShouldDrawTabStrip() const {
+  if (agent_mode_ == AgentWindowMode::kAgentOnly) {
+    return false;
+  }
+
   // Return false if this window does not normally display a tabstrip or if the
   // tabstrip is currently hidden, e.g. because we're in fullscreen.
   if (!browser_->SupportsWindowFeature(Browser::FEATURE_TABSTRIP)) {
@@ -3286,6 +3363,10 @@ bool BrowserView::IsTabStripEditable() const {
 }
 
 bool BrowserView::IsToolbarVisible() const {
+  if (agent_mode_ == AgentWindowMode::kAgentOnly) {
+    return false;
+  }
+
 #if BUILDFLAG(IS_MAC)
   // Immersive full screen makes it possible to display the toolbar when
   // kShowFullscreenToolbar is not set.
@@ -4985,6 +5066,18 @@ int BrowserView::NonClientHitTest(const gfx::Point& point) {
   views::View::ConvertPointToTarget(parent(), this,
                                     &point_in_browser_view_coords);
 
+  if (agent_pane_ && agent_pane_->GetVisible() &&
+      agent_pane_->bounds().Contains(point_in_browser_view_coords) &&
+      point_in_browser_view_coords.y() <
+          agent_pane_->y() + AgentPaneView::kTitleBarHeight) {
+    // Let the non-client frame/AppKit resolve title-strip background to a
+    // draggable caption. Resolve controls here, before Chromium's full-width
+    // tab-strip shadow branch can misclassify them as draggable.
+    return agent_pane_->IsPointInDragStrip(point_in_browser_view_coords)
+               ? HTNOWHERE
+               : HTCLIENT;
+  }
+
   // Check if the point is in the web_app_frame_toolbar_. Because this toolbar
   // can entirely be within the window controls overlay area, this check needs
   // to be done before the window controls overlay area check below.
@@ -5248,7 +5341,7 @@ void BrowserView::AddedToWidget() {
           infobar_container_, contents_container_,
           left_aligned_side_panel_separator_, unified_side_panel_,
           right_aligned_side_panel_separator_, side_panel_rounded_corner_,
-          immersive_mode_controller_.get(), contents_separator_));
+          immersive_mode_controller_.get(), contents_separator_, agent_pane_));
   browser_view_layout->SetUseBrowserContentMinimumSize(
       ShouldUseBrowserContentMinimumSize());
 
