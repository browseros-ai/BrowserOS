diff --git a/chrome/browser/ui/views/frame/browser_view.h b/chrome/browser/ui/views/frame/browser_view.h
index 8a06d49d2abe1258a3c6723fcfe682e68cc90fe0..d0643618557e61bd6f1f5a373d3cf2d08552780c 100644
--- a/chrome/browser/ui/views/frame/browser_view.h
+++ b/chrome/browser/ui/views/frame/browser_view.h
@@ -69,6 +69,7 @@
 // view: http://dev.chromium.org/developers/design-documents/browser-window
 
 class AccessibilityFocusHighlight;
+class AgentPaneView;
 class BookmarkBarView;
 class Browser;
 class ContentsLayoutManager;
@@ -148,6 +149,14 @@ class BrowserView : public BrowserWindow,
   METADATA_HEADER(BrowserView, views::ClientView)
 
  public:
+  enum class AgentWindowMode {
+    kDisabled,
+    kAgentOnly,
+    kSplit,
+  };
+
+  static constexpr int kAgentTitleBarHeight = 36;
+
   explicit BrowserView(std::unique_ptr<Browser> browser);
   BrowserView(const BrowserView&) = delete;
   BrowserView& operator=(const BrowserView&) = delete;
@@ -244,6 +253,12 @@ class BrowserView : public BrowserWindow,
 
   SidePanel* unified_side_panel() { return unified_side_panel_; }
 
+  AgentWindowMode agent_window_mode() const { return agent_mode_; }
+  AgentPaneView* agent_pane() { return agent_pane_; }
+  const AgentPaneView* agent_pane() const { return agent_pane_; }
+  void SetAgentWindowMode(AgentWindowMode mode);
+  void ToggleAgentSplit();
+
   void set_contents_border_widget(views::Widget* contents_border_widget) {
     GetBrowserViewLayout()->set_contents_border_widget(contents_border_widget);
   }
@@ -1323,6 +1338,11 @@ class BrowserView : public BrowserWindow,
   raw_ptr<views::View> left_aligned_side_panel_separator_ = nullptr;
   raw_ptr<views::View> side_panel_rounded_corner_ = nullptr;
 
+  // BrowserOS agent-first window surface. Normal windows start with the agent
+  // filling the window and may toggle Chromium into a right-hand split.
+  raw_ptr<AgentPaneView> agent_pane_ = nullptr;
+  AgentWindowMode agent_mode_ = AgentWindowMode::kDisabled;
+
   // Provides access to the toolbar buttons this browser view uses. Buttons may
   // appear in a hosted app frame or in a tabbed UI toolbar.
   raw_ptr<ToolbarButtonProvider> toolbar_button_provider_ = nullptr;
