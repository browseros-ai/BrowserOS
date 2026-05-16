diff --git a/chrome/browser/ui/views/frame/multi_contents_view.h b/chrome/browser/ui/views/frame/multi_contents_view.h
index e24b980a8fb42..b9fe34148edba 100644
--- a/chrome/browser/ui/views/frame/multi_contents_view.h
+++ b/chrome/browser/ui/views/frame/multi_contents_view.h
@@ -16,6 +16,7 @@
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/contents_container_view.h"
 #include "components/prefs/pref_change_registrar.h"
+#include "components/split_tabs/split_tab_visual_data.h"
 #include "ui/base/interaction/element_identifier.h"
 #include "ui/base/metadata/metadata_header_macros.h"
 #include "ui/views/controls/resize_area_delegate.h"
@@ -56,10 +57,10 @@ class MultiContentsView
  public:
   using FocusableViewMap = base::flat_map<std::string, views::View*>;
 
-  struct ViewWidths {
-    double start_width = 0;
-    double resize_width = 0;
-    double end_width = 0;
+  struct ViewSizes {
+    double start_size = 0;
+    double resize_size = 0;
+    double end_size = 0;
   };
 
   static constexpr int kSplitViewContentInset = 8;
@@ -90,7 +91,7 @@ class MultiContentsView
   // Show the split view without set any WebContents and update the size of
   // contents views based on `ratio`, this is used to prepare the layout and
   // prevent a re-layout of WebContents.
-  void ShowSplitView(double ratio);
+  void ShowSplitView(double ratio, split_tabs::SplitTabLayout split_layout);
 
   // Preserves the active WebContents and hides the second ContentsContainerView
   // and resize handle.
@@ -108,6 +109,8 @@ class MultiContentsView
 
   // Updates the size of the contents views based on |ratio|.
   void UpdateSplitRatio(double ratio);
+  void UpdateSplitVisualData(double ratio,
+                             split_tabs::SplitTabLayout split_layout);
   double GetSplitRatio() const { return start_ratio_; }
 
   // SplitTabHighlightController::Delegate:
@@ -147,7 +150,7 @@ class MultiContentsView
 
   // If the split view is being resized.
   bool IsSplitResizing() const {
-    return initial_start_width_on_resize_.has_value();
+    return initial_start_size_on_resize_.has_value();
   }
 
   // Returns accessible panes to be used in BrowserView to create the order of
@@ -252,17 +255,20 @@ class MultiContentsView
   void OnReadAnythingOverlayFocused(ContentsContainerView* container,
                                     views::WebView* web_view);
 
-  ViewWidths GetViewWidths(gfx::Rect available_space) const;
+  ViewSizes GetViewSizes(gfx::Rect available_space) const;
 
   // Clamps to the minimum of kMinWebContentsWidth or
-  // kMinWebContentsWidthPercentage multiplied by the available width. This
-  // allows for some flexibility when it comes to particularly narrow windows.
-  ViewWidths ClampToMinWidth(gfx::Rect available_space,
-                             ViewWidths widths) const;
+  // kMinWebContentsWidthPercentage multiplied by the available split axis size.
+  // This allows for some flexibility when it comes to particularly narrow or
+  // short windows.
+  ViewSizes ClampToMinViewSize(gfx::Rect available_space,
+                               ViewSizes sizes) const;
 
-  // Returns the minimum width for a single view within the `MultiContentsView`.
+  // Returns the minimum size for a single view within the `MultiContentsView`.
   // Returns 0 if not in a split view.
-  int GetMinViewWidth(gfx::Rect available_space) const;
+  int GetMinViewSize(gfx::Rect available_space) const;
+
+  void UpdateContentsViewInsets();
 
   void UpdateContentsBorderAndOverlay();
 
@@ -299,16 +305,20 @@ class MultiContentsView
   // The index in contents_views_ of the active contents view.
   int active_index_ = 0;
 
-  // Current ratio of |contents_views_|'s first ContentsContainerView's width /
-  // overall contents view width.
+  // Current ratio of |contents_views_|'s first ContentsContainerView's size /
+  // overall contents view size along the active split axis.
   double start_ratio_ = 0.5;
 
   // See `SetTargetContentBounds()`.
   std::optional<TargetContentBounds> target_content_bounds_;
 
-  // Width of `start_contents_.contents_view_` when a resize action began.
+  split_tabs::SplitTabLayout split_layout_ =
+      split_tabs::SplitTabLayout::kVertical;
+
+  // Size of `start_contents_.contents_view_` along the active split axis when a
+  // resize action began.
   // Nullopt if not currently resizing.
-  std::optional<double> initial_start_width_on_resize_;
+  std::optional<double> initial_start_size_on_resize_;
 
   // Insets of the start and end contents view when in split view
   gfx::Insets start_contents_view_inset_;
