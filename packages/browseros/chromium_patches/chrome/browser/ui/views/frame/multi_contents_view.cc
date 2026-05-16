diff --git a/chrome/browser/ui/views/frame/multi_contents_view.cc b/chrome/browser/ui/views/frame/multi_contents_view.cc
index aaec9ccc448c9..2731a0a40c00b 100644
--- a/chrome/browser/ui/views/frame/multi_contents_view.cc
+++ b/chrome/browser/ui/views/frame/multi_contents_view.cc
@@ -37,6 +37,7 @@
 #include "chrome/common/pref_names.h"
 #include "chrome/common/webui_url_constants.h"
 #include "components/prefs/pref_service.h"
+#include "components/split_tabs/split_tab_visual_data.h"
 #include "content/public/browser/web_contents.h"
 #include "content/public/common/url_constants.h"
 #include "ui/base/metadata/metadata_impl_macros.h"
@@ -65,12 +66,7 @@ void MultiContentsView::ContentsSeparators::Reset() {
 MultiContentsView::MultiContentsView(
     BrowserView* browser_view,
     std::unique_ptr<MultiContentsViewDelegate> delegate)
-    : browser_view_(browser_view),
-      delegate_(std::move(delegate)),
-      start_contents_view_inset_(
-          gfx::Insets(kSplitViewContentInset).set_top(0).set_right(0)),
-      end_contents_view_inset_(
-          gfx::Insets(kSplitViewContentInset).set_top(0).set_left(0)) {
+    : browser_view_(browser_view), delegate_(std::move(delegate)) {
   SetLayoutManager(std::make_unique<views::DelegatingLayoutManager>(this));
   SetProperty(views::kElementIdentifierKey, kMultiContentsViewElementId);
 
@@ -119,6 +115,7 @@ MultiContentsView::MultiContentsView(
           kColorToolbarContentAreaSeparator));
   contents_separators_.corner_separator->SetProperty(
       views::kElementIdentifierKey, kContentsSeparatorTopCornerElementId);
+  UpdateContentsViewInsets();
 
   for (auto* contents_container_view : contents_container_views_) {
     auto& view_map = container_focusable_map_[contents_container_view];
@@ -246,20 +243,15 @@ void MultiContentsView::SetWebContentsAtIndex(
   }
 }
 
-void MultiContentsView::ShowSplitView(double ratio) {
+void MultiContentsView::ShowSplitView(double ratio,
+                                      split_tabs::SplitTabLayout split_layout) {
+  UpdateSplitVisualData(ratio, split_layout);
   if (!contents_container_views_[1]->GetVisible()) {
-    // If split view is not visible, set the `start_ratio_` and update the view
-    // visibility.
-    start_ratio_ = ratio;
+    // If split view is not visible, update the view visibility.
     contents_container_views_[1]->SetVisible(true);
     resize_area_->SetVisible(true);
     UpdateContentsBorderAndOverlay();
-  } else if (start_ratio_ != ratio) {
-    // If the split view is visible but ratio is changed, update the split
-    // ratio.
-    UpdateSplitRatio(ratio);
   }
-  // Split view is visible and ratio is not changed, do nothing.
 }
 
 void MultiContentsView::CloseSplitView() {
@@ -305,11 +297,29 @@ void MultiContentsView::SetActiveIndex(int index) {
 }
 
 void MultiContentsView::UpdateSplitRatio(double ratio) {
-  if (start_ratio_ == ratio) {
+  UpdateSplitVisualData(ratio, split_layout_);
+}
+
+void MultiContentsView::UpdateSplitVisualData(
+    double ratio,
+    split_tabs::SplitTabLayout split_layout) {
+  bool needs_layout = false;
+  if (split_layout_ != split_layout) {
+    split_layout_ = split_layout;
+    resize_area_->SetSplitLayout(split_layout_);
+    UpdateContentsViewInsets();
+    needs_layout = true;
+  }
+
+  if (start_ratio_ != ratio) {
+    start_ratio_ = ratio;
+    needs_layout = true;
+  }
+
+  if (!needs_layout) {
     return;
   }
 
-  start_ratio_ = ratio;
   InvalidateLayout();
 }
 
@@ -347,24 +357,36 @@ std::vector<views::View*> MultiContentsView::GetAccessiblePanes() {
 }
 
 void MultiContentsView::OnResize(int resize_amount, bool done_resizing) {
-  if (!initial_start_width_on_resize_.has_value()) {
-    initial_start_width_on_resize_ =
-        std::make_optional(contents_container_views_[0]->size().width());
-  }
-  double total_width = contents_container_views_[0]->size().width() +
-                       contents_container_views_[0]->GetInsets().width() +
-                       contents_container_views_[1]->size().width() +
-                       contents_container_views_[1]->GetInsets().width();
-  double end_width = (initial_start_width_on_resize_.value() +
-                      contents_container_views_[0]->GetInsets().width() +
-                      static_cast<double>(resize_amount));
-
-  // If end_width is within the snap point widths, update to the snap point.
+  const bool is_horizontal_split =
+      split_layout_ == split_tabs::SplitTabLayout::kHorizontal;
+  if (!initial_start_size_on_resize_.has_value()) {
+    initial_start_size_on_resize_ = std::make_optional(
+        is_horizontal_split ? contents_container_views_[0]->size().height()
+                            : contents_container_views_[0]->size().width());
+  }
+  const double total_size =
+      (is_horizontal_split ? contents_container_views_[0]->size().height()
+                           : contents_container_views_[0]->size().width()) +
+      (is_horizontal_split
+           ? contents_container_views_[0]->GetInsets().height()
+           : contents_container_views_[0]->GetInsets().width()) +
+      (is_horizontal_split ? contents_container_views_[1]->size().height()
+                           : contents_container_views_[1]->size().width()) +
+      (is_horizontal_split ? contents_container_views_[1]->GetInsets().height()
+                           : contents_container_views_[1]->GetInsets().width());
+  const double start_size =
+      initial_start_size_on_resize_.value() +
+      (is_horizontal_split
+           ? contents_container_views_[0]->GetInsets().height()
+           : contents_container_views_[0]->GetInsets().width()) +
+      static_cast<double>(resize_amount);
+
+  // If start_size is within the snap point sizes, update to the snap point.
   delegate_->ResizeWebContents(
-      CalculateRatioWithSnapPoints(end_width, total_width), done_resizing);
+      CalculateRatioWithSnapPoints(start_size, total_size), done_resizing);
 
   if (done_resizing) {
-    initial_start_width_on_resize_ = std::nullopt;
+    initial_start_size_on_resize_ = std::nullopt;
   }
 }
 
@@ -464,15 +486,30 @@ views::ProposedLayout MultiContentsView::CalculateProposedLayout(
   available_space =
       CalculateSeparatorLayouts(available_space, layouts.child_layouts);
 
-  ViewWidths widths = GetViewWidths(available_space);
-
-  gfx::Rect start_rect(available_space.origin(),
-                       gfx::Size(widths.start_width, available_space.height()));
-  gfx::Rect resize_rect(
-      start_rect.top_right(),
-      gfx::Size(widths.resize_width, available_space.height()));
-  gfx::Rect end_rect(resize_rect.top_right(),
-                     gfx::Size(widths.end_width, available_space.height()));
+  ViewSizes sizes = GetViewSizes(available_space);
+
+  gfx::Rect start_rect;
+  gfx::Rect resize_rect;
+  gfx::Rect end_rect;
+  if (split_layout_ == split_tabs::SplitTabLayout::kHorizontal) {
+    start_rect =
+        gfx::Rect(available_space.origin(),
+                  gfx::Size(available_space.width(), sizes.start_size));
+    resize_rect =
+        gfx::Rect(gfx::Point(available_space.x(), start_rect.bottom()),
+                  gfx::Size(available_space.width(), sizes.resize_size));
+    end_rect = gfx::Rect(gfx::Point(available_space.x(), resize_rect.bottom()),
+                         gfx::Size(available_space.width(), sizes.end_size));
+  } else {
+    start_rect =
+        gfx::Rect(available_space.origin(),
+                  gfx::Size(sizes.start_size, available_space.height()));
+    resize_rect =
+        gfx::Rect(start_rect.top_right(),
+                  gfx::Size(sizes.resize_size, available_space.height()));
+    end_rect = gfx::Rect(resize_rect.top_right(),
+                         gfx::Size(sizes.end_size, available_space.height()));
+  }
 
   if (IsInSplitView()) {
     start_rect.Inset(start_contents_view_inset_);
@@ -605,61 +642,89 @@ gfx::Rect MultiContentsView::CalculateSeparatorLayouts(
                    height - separator_height);
 }
 
-MultiContentsView::ViewWidths MultiContentsView::GetViewWidths(
+MultiContentsView::ViewSizes MultiContentsView::GetViewSizes(
     gfx::Rect available_space) const {
-  ViewWidths widths;
+  ViewSizes sizes;
+  const bool is_horizontal_split =
+      split_layout_ == split_tabs::SplitTabLayout::kHorizontal;
+  const int available_size =
+      is_horizontal_split ? available_space.height() : available_space.width();
   if (IsInSplitView()) {
     CHECK(contents_container_views_[0]->GetVisible() &&
           contents_container_views_[1]->GetVisible());
-    widths.resize_width = resize_area_->GetPreferredSize().width();
-    widths.start_width =
-        start_ratio_ * (available_space.width() - widths.resize_width);
-    widths.end_width =
-        available_space.width() - widths.start_width - widths.resize_width;
+    sizes.resize_size = is_horizontal_split
+                            ? resize_area_->GetPreferredSize().height()
+                            : resize_area_->GetPreferredSize().width();
+    sizes.start_size = start_ratio_ * (available_size - sizes.resize_size);
+    sizes.end_size = available_size - sizes.start_size - sizes.resize_size;
   } else {
     CHECK(!contents_container_views_[1]->GetVisible());
-    widths.start_width = available_space.width();
+    sizes.start_size = available_size;
   }
-  return ClampToMinWidth(available_space, widths);
+  return ClampToMinViewSize(available_space, sizes);
 }
 
-MultiContentsView::ViewWidths MultiContentsView::ClampToMinWidth(
+MultiContentsView::ViewSizes MultiContentsView::ClampToMinViewSize(
     gfx::Rect available_space,
-    ViewWidths widths) const {
+    ViewSizes sizes) const {
   if (!IsInSplitView()) {
     // Don't clamp if in a single-view state, where other views should be 0
-    // width.
-    return widths;
+    // size.
+    return sizes;
   }
 
-  const int min_width = GetMinViewWidth(available_space);
-  if (widths.start_width < min_width) {
-    const double diff = min_width - widths.start_width;
-    widths.start_width += diff;
-    widths.end_width -= diff;
-  } else if (widths.end_width < min_width) {
-    const double diff = min_width - widths.end_width;
-    widths.end_width += diff;
-    widths.start_width -= diff;
+  const int min_size = GetMinViewSize(available_space);
+  if (sizes.start_size < min_size) {
+    const double diff = min_size - sizes.start_size;
+    sizes.start_size += diff;
+    sizes.end_size -= diff;
+  } else if (sizes.end_size < min_size) {
+    const double diff = min_size - sizes.end_size;
+    sizes.end_size += diff;
+    sizes.start_size -= diff;
   }
-  return widths;
+  return sizes;
 }
 
-int MultiContentsView::GetMinViewWidth(gfx::Rect available_space) const {
+int MultiContentsView::GetMinViewSize(gfx::Rect available_space) const {
   CHECK(IsInSplitView());
 
-  // The minimum width for a content view in a split should be the lesser of
+  // The minimum size for a content view in a split should be the lesser of
   // kMinWebContentsWidth, and kMinWebContentsWidthPercentage as a percentage of
-  // the MultiContentsView's available width with a lower bound of
+  // the MultiContentsView's available split axis size with a lower bound of
   // kConstrainedMinWebContentsWidth.
-  const int min_percentage =
-      kMinWebContentsWidthPercentage * available_space.width();
+  const int available_size =
+      split_layout_ == split_tabs::SplitTabLayout::kHorizontal
+          ? available_space.height()
+          : available_space.width();
+  const int min_percentage = kMinWebContentsWidthPercentage * available_size;
   const int min_fixed_value =
       min_contents_width_for_testing_.value_or(kMinWebContentsWidth);
   return std::min(min_fixed_value,
                   std::max(kConstrainedMinWebContentsWidth, min_percentage));
 }
 
+void MultiContentsView::UpdateContentsViewInsets() {
+  const int top_inset =
+      contents_separators_.should_show_top ? 0 : kSplitViewContentInset;
+  const int leading_inset =
+      contents_separators_.should_show_leading ? 0 : kSplitViewContentInset;
+  const int trailing_inset =
+      contents_separators_.should_show_trailing ? 0 : kSplitViewContentInset;
+
+  if (split_layout_ == split_tabs::SplitTabLayout::kHorizontal) {
+    start_contents_view_inset_ =
+        gfx::Insets::TLBR(top_inset, leading_inset, 0, trailing_inset);
+    end_contents_view_inset_ = gfx::Insets::TLBR(
+        0, leading_inset, kSplitViewContentInset, trailing_inset);
+  } else {
+    start_contents_view_inset_ =
+        gfx::Insets::TLBR(top_inset, leading_inset, kSplitViewContentInset, 0);
+    end_contents_view_inset_ =
+        gfx::Insets::TLBR(top_inset, 0, kSplitViewContentInset, trailing_inset);
+  }
+}
+
 void MultiContentsView::UpdateContentsBorderAndOverlay() {
   for (auto* contents_container_view : contents_container_views_) {
     const bool is_active =
@@ -709,10 +774,7 @@ void MultiContentsView::SetShouldShowTopSeparator(bool should_show) {
     return;
   }
   contents_separators_.should_show_top = should_show;
-  start_contents_view_inset_.set_top(
-      should_show ? 0 : MultiContentsView::kSplitViewContentInset);
-  end_contents_view_inset_.set_top(
-      should_show ? 0 : MultiContentsView::kSplitViewContentInset);
+  UpdateContentsViewInsets();
 
   // This can be called during BrowserView layout, so protect against creating a
   // layout loop.
@@ -724,8 +786,7 @@ void MultiContentsView::SetShouldShowLeadingSeparator(bool should_show) {
     return;
   }
   contents_separators_.should_show_leading = should_show;
-  start_contents_view_inset_.set_left(
-      should_show ? 0 : MultiContentsView::kSplitViewContentInset);
+  UpdateContentsViewInsets();
 
   // This can be called during BrowserView layout, so protect against creating a
   // layout loop.
@@ -737,8 +798,7 @@ void MultiContentsView::SetShouldShowTrailingSeparator(bool should_show) {
     return;
   }
   contents_separators_.should_show_trailing = should_show;
-  end_contents_view_inset_.set_right(
-      should_show ? 0 : MultiContentsView::kSplitViewContentInset);
+  UpdateContentsViewInsets();
 
   // This can be called during BrowserView layout, so protect against creating a
   // layout loop.
