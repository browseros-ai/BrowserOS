diff --git a/chrome/browser/ui/views/frame/browser_view_layout_unittest.cc b/chrome/browser/ui/views/frame/browser_view_layout_unittest.cc
index 5a8bec0aecd482ac0eb28ac26b4521800ef6b42e..5354337f96bc9fd3a6b34befe8c1dab9388ed9ab 100644
--- a/chrome/browser/ui/views/frame/browser_view_layout_unittest.cc
+++ b/chrome/browser/ui/views/frame/browser_view_layout_unittest.cc
@@ -8,6 +8,7 @@
 
 #include "base/containers/fixed_flat_set.h"
 #include "base/memory/raw_ptr.h"
+#include "chrome/browser/ui/views/frame/agent_pane_view.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/frame/browser_view_layout_delegate.h"
 #include "chrome/browser/ui/views/frame/contents_layout_manager.h"
@@ -41,6 +42,13 @@ class MockBrowserViewLayoutDelegate : public BrowserViewLayoutDelegate {
     should_draw_tab_strip_ = visible;
   }
   void set_toolbar_visible(bool visible) { toolbar_visible_ = visible; }
+  void set_agent_pane_width(int width) { agent_pane_width_ = width; }
+  void set_agent_pane_only(bool agent_pane_only) {
+    agent_pane_only_ = agent_pane_only;
+  }
+  void set_tab_strip_bounds(const gfx::Rect& bounds) {
+    tab_strip_bounds_ = bounds;
+  }
   void set_bookmark_bar_visible(bool visible) {
     bookmark_bar_visible_ = visible;
   }
@@ -58,7 +66,7 @@ class MockBrowserViewLayoutDelegate : public BrowserViewLayoutDelegate {
   bool ShouldDrawTabStrip() const override { return should_draw_tab_strip_; }
   bool GetBorderlessModeEnabled() const override { return false; }
   gfx::Rect GetBoundsForTabStripRegionInBrowserView() const override {
-    return gfx::Rect();
+    return tab_strip_bounds_;
   }
   gfx::Rect GetBoundsForWebAppFrameToolbarInBrowserView() const override {
     return gfx::Rect();
@@ -68,6 +76,8 @@ class MockBrowserViewLayoutDelegate : public BrowserViewLayoutDelegate {
       views::Label& window_title_label) const override {}
   int GetTopInsetInBrowserView() const override { return 0; }
   bool IsToolbarVisible() const override { return toolbar_visible_; }
+  int GetAgentPaneWidth() const override { return agent_pane_width_; }
+  bool IsAgentPaneOnly() const override { return agent_pane_only_; }
   bool IsBookmarkBarVisible() const override { return bookmark_bar_visible_; }
   bool IsContentsSeparatorEnabled() const override {
     return content_separator_enabled_;
@@ -105,6 +115,9 @@ class MockBrowserViewLayoutDelegate : public BrowserViewLayoutDelegate {
  private:
   bool should_draw_tab_strip_ = true;
   bool toolbar_visible_ = true;
+  int agent_pane_width_ = 0;
+  bool agent_pane_only_ = false;
+  gfx::Rect tab_strip_bounds_;
   bool bookmark_bar_visible_ = true;
   bool content_separator_enabled_ = true;
   bool top_controls_slide_enabled_ = false;
@@ -160,6 +173,7 @@ class BrowserViewLayoutTest : public ChromeViewsTestBase {
         toolbar_(nullptr),
         infobar_container_(nullptr),
         contents_container_(nullptr),
+        agent_pane_(nullptr),
         contents_web_view_(nullptr),
         devtools_web_view_(nullptr) {}
 
@@ -178,12 +192,16 @@ class BrowserViewLayoutTest : public ChromeViewsTestBase {
   views::View* separator() { return separator_; }
   InfoBarContainerView* infobar_container() { return infobar_container_; }
   views::View* contents_container() { return contents_container_; }
+  AgentPaneView* agent_pane() { return agent_pane_; }
 
   void SetUp() override {
     ChromeViewsTestBase::SetUp();
 
     browser_view_ = CreateFixedSizeView(gfx::Size(800, 600));
 
+    agent_pane_ = browser_view_->AddChildView(
+        std::make_unique<AgentPaneView>(/*browser_view=*/nullptr));
+
     immersive_mode_controller_ =
         std::make_unique<MockImmersiveModeController>();
 
@@ -240,7 +258,7 @@ class BrowserViewLayoutTest : public ChromeViewsTestBase {
         /*unified_side_panel=*/nullptr,
         /*right_aligned_side_panel_separator=*/nullptr,
         side_panel_rounded_corner_, immersive_mode_controller_.get(),
-        separator_);
+        separator_, agent_pane_);
     layout->set_webui_tab_strip(webui_tab_strip());
     layout_ = layout.get();
     browser_view_->SetLayoutManager(std::move(layout));
@@ -277,6 +295,7 @@ class BrowserViewLayoutTest : public ChromeViewsTestBase {
   raw_ptr<InfoBarContainerView> infobar_container_;
   raw_ptr<views::View> side_panel_rounded_corner_;
   raw_ptr<views::View> contents_container_;
+  raw_ptr<AgentPaneView> agent_pane_;
   raw_ptr<views::View> contents_web_view_;
   raw_ptr<views::View> devtools_web_view_;
   raw_ptr<views::View> devtools_scrim_view_;
@@ -333,6 +352,77 @@ TEST_F(BrowserViewLayoutTest, Layout) {
   // TODO(jamescook): Tab strip and bookmark bar.
 }
 
+TEST_F(BrowserViewLayoutTest, AgentOnlyPreservesBrowserContents) {
+  delegate()->set_agent_pane_width(800);
+  delegate()->set_agent_pane_only(true);
+  delegate()->set_should_draw_tab_strip(false);
+  delegate()->set_toolbar_visible(false);
+  delegate()->set_bookmark_bar_visible(false);
+  InvalidateAndRunScheduledLayoutOnBrowserView();
+
+  EXPECT_TRUE(agent_pane()->GetVisible());
+  EXPECT_EQ(gfx::Rect(0, 0, 800, 600), agent_pane()->bounds());
+  EXPECT_FALSE(contents_container()->GetVisible());
+
+  delegate()->set_agent_pane_width(320);
+  delegate()->set_agent_pane_only(false);
+  InvalidateAndRunScheduledLayoutOnBrowserView();
+
+  EXPECT_TRUE(contents_container()->GetVisible());
+  EXPECT_EQ(gfx::Rect(320, 0, 480, 600), contents_container()->bounds());
+}
+
+TEST_F(BrowserViewLayoutTest, SplitUsesRightColumnCoordinatesAndMinimumWidth) {
+  const int original_minimum_width =
+      layout()->GetMinimumSize(browser_view()).width();
+  delegate()->set_agent_pane_width(320);
+  delegate()->set_tab_strip_bounds(gfx::Rect(72, 0, 728, 30));
+
+  std::unique_ptr<views::View> download_shelf =
+      CreateFixedSizeView(gfx::Size(800, 50));
+  download_shelf->SetVisible(true);
+  layout()->set_download_shelf(download_shelf.get());
+  InvalidateAndRunScheduledLayoutOnBrowserView();
+
+  EXPECT_EQ(gfx::Rect(0, 0, 320, 600), agent_pane()->bounds());
+  EXPECT_EQ(320, top_container()->x());
+  EXPECT_EQ(480, top_container()->width());
+  EXPECT_EQ(0, tab_strip()->parent()->x());
+  EXPECT_EQ(480, tab_strip()->parent()->width());
+  EXPECT_EQ(0, toolbar()->x());
+  EXPECT_EQ(480, toolbar()->width());
+  EXPECT_EQ(0, separator()->x());
+  EXPECT_EQ(480, separator()->width());
+  EXPECT_EQ(320, infobar_container()->x());
+  EXPECT_EQ(480, infobar_container()->width());
+  EXPECT_EQ(320, contents_container()->x());
+  EXPECT_EQ(480, contents_container()->width());
+  EXPECT_EQ(gfx::Rect(320, 550, 480, 50), download_shelf->bounds());
+  EXPECT_EQ(original_minimum_width + AgentPaneView::kMinimumWidth +
+                AgentPaneView::kResizeAreaWidth,
+            layout()->GetMinimumSize(browser_view()).width());
+
+  webui_tab_strip()->SetVisible(true);
+  InvalidateAndRunScheduledLayoutOnBrowserView();
+  EXPECT_EQ(0, webui_tab_strip()->x());
+  EXPECT_EQ(480, webui_tab_strip()->width());
+  webui_tab_strip()->SetVisible(false);
+
+  // The coordinate helper must consult the current parent at layout time.
+  // BookmarkBarView performs this same reparenting in production.
+  browser_view()->AddChildView(top_container()->RemoveChildViewT(toolbar()));
+  InvalidateAndRunScheduledLayoutOnBrowserView();
+  EXPECT_EQ(320, toolbar()->x());
+  EXPECT_EQ(480, toolbar()->width());
+
+  agent_pane()->OnResize(-1000, /*done_resizing=*/true);
+  EXPECT_EQ(AgentPaneView::kMinimumWidth, agent_pane()->split_width());
+  agent_pane()->OnResize(80, /*done_resizing=*/true);
+  EXPECT_EQ(400, agent_pane()->split_width());
+
+  layout()->set_download_shelf(nullptr);
+}
+
 TEST_F(BrowserViewLayoutTest, LayoutDownloadShelf) {
   constexpr int kHeight = 50;
   std::unique_ptr<views::View> download_shelf =
