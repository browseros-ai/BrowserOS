diff --git a/chrome/browser/ui/views/frame/multi_contents_resize_area.cc b/chrome/browser/ui/views/frame/multi_contents_resize_area.cc
index 72347969217f6..d7cf9d21f61ac 100644
--- a/chrome/browser/ui/views/frame/multi_contents_resize_area.cc
+++ b/chrome/browser/ui/views/frame/multi_contents_resize_area.cc
@@ -11,11 +11,13 @@
 #include "chrome/browser/ui/views/frame/multi_contents_view.h"
 #include "chrome/grit/generated_resources.h"
 #include "ui/accessibility/mojom/ax_node_data.mojom.h"
+#include "ui/base/cursor/cursor.h"
 #include "ui/base/l10n/l10n_util.h"
 #include "ui/base/metadata/metadata_impl_macros.h"
 #include "ui/color/color_provider.h"
 #include "ui/compositor/layer.h"
 #include "ui/compositor/layer_type.h"
+#include "ui/gfx/geometry/point.h"
 #include "ui/gfx/geometry/size.h"
 #include "ui/views/accessibility/view_accessibility.h"
 #include "ui/views/background.h"
@@ -57,6 +59,15 @@ MultiContentsResizeHandle::MultiContentsResizeHandle() {
       kColorSidePanelHoverResizeAreaHandle, kHandleCornerRadius));
 }
 
+void MultiContentsResizeHandle::SetSplitLayout(
+    split_tabs::SplitTabLayout split_layout) {
+  if (split_layout == split_tabs::SplitTabLayout::kHorizontal) {
+    SetPreferredSize(gfx::Size(kHandleHeight, kHandleWidth));
+  } else {
+    SetPreferredSize(gfx::Size(kHandleWidth, kHandleHeight));
+  }
+}
+
 void MultiContentsResizeHandle::UpdateVisibility() {
   layer()->SetVisible(parent()->GetVisible() &&
                       (HasFocus() || parent()->IsMouseHovered()));
@@ -91,31 +102,131 @@ MultiContentsResizeArea::MultiContentsResizeArea(
   resize_handle_ = AddChildView(std::make_unique<MultiContentsResizeHandle>());
 
   SetProperty(views::kElementIdentifierKey, kMultiContentsResizeAreaElementId);
-  SetPreferredSize(gfx::Size(kHandleWidth + kHandlePadding, kHandleHeight));
+  SetSplitLayout(split_tabs::SplitTabLayout::kVertical);
+}
+
+void MultiContentsResizeArea::SetSplitLayout(
+    split_tabs::SplitTabLayout split_layout) {
+  split_layout_ = split_layout;
+  auto* layout_manager = static_cast<views::FlexLayout*>(GetLayoutManager());
+  if (IsHorizontalSplit()) {
+    layout_manager->SetOrientation(views::LayoutOrientation::kHorizontal);
+    SetPreferredSize(gfx::Size(kHandleHeight, kHandleWidth + kHandlePadding));
+  } else {
+    layout_manager->SetOrientation(views::LayoutOrientation::kVertical);
+    SetPreferredSize(gfx::Size(kHandleWidth + kHandlePadding, kHandleHeight));
+  }
+  resize_handle_->SetSplitLayout(split_layout_);
+  InvalidateLayout();
+}
+
+ui::Cursor MultiContentsResizeArea::GetCursor(const ui::MouseEvent& event) {
+  if (!GetEnabled()) {
+    return ui::Cursor();
+  }
+
+  if (IsHorizontalSplit()) {
+    return ui::Cursor(ui::mojom::CursorType::kNorthSouthResize);
+  }
+  return ResizeArea::GetCursor(event);
 }
 
 void MultiContentsResizeArea::OnGestureEvent(ui::GestureEvent* event) {
   // If the gesture event was a double tap and was not part of a resizing event,
   // swap the contents views.
-  if (!is_resizing() && event->type() == ui::EventType::kGestureTap &&
+  if (!IsResizeInProgress() && event->type() == ui::EventType::kGestureTap &&
       event->details().tap_count() == 2) {
     multi_contents_view_->OnSwap();
   }
+
+  if (IsHorizontalSplit()) {
+    if (event->type() == ui::EventType::kGestureTapDown) {
+      SetInitialResizePosition(event->y());
+      event->SetHandled();
+    } else if (event->type() == ui::EventType::kGestureScrollBegin ||
+               event->type() == ui::EventType::kGestureScrollUpdate) {
+      ReportResizeAmount(event->y(), false);
+      event->SetHandled();
+    } else if (event->type() == ui::EventType::kGestureEnd) {
+      if (is_resizing_horizontally_) {
+        ReportResizeAmount(event->y(), true);
+      }
+      event->SetHandled();
+    }
+    return;
+  }
+
   ResizeArea::OnGestureEvent(event);
 }
 
+bool MultiContentsResizeArea::OnMousePressed(const ui::MouseEvent& event) {
+  if (!IsHorizontalSplit()) {
+    return ResizeArea::OnMousePressed(event);
+  }
+
+  if (!event.IsOnlyLeftMouseButton()) {
+    return false;
+  }
+
+  SetInitialResizePosition(event.y());
+  return true;
+}
+
+bool MultiContentsResizeArea::OnMouseDragged(const ui::MouseEvent& event) {
+  if (!IsHorizontalSplit()) {
+    return ResizeArea::OnMouseDragged(event);
+  }
+
+  if (!event.IsLeftMouseButton()) {
+    return false;
+  }
+
+  ReportResizeAmount(event.y(), false);
+  return true;
+}
+
 void MultiContentsResizeArea::OnMouseReleased(const ui::MouseEvent& event) {
   // If the mouse event was a left double click and was not part of a resizing
   // event, swap the contents views.
-  if (!is_resizing() && event.IsOnlyLeftMouseButton() &&
+  if (!IsResizeInProgress() && event.IsOnlyLeftMouseButton() &&
       event.GetClickCount() == 2) {
     multi_contents_view_->OnSwap();
   }
+
+  if (IsHorizontalSplit()) {
+    if (is_resizing_horizontally_) {
+      ReportResizeAmount(event.y(), true);
+    }
+    return;
+  }
+
   ResizeArea::OnMouseReleased(event);
 }
 
+void MultiContentsResizeArea::OnMouseCaptureLost() {
+  if (!IsHorizontalSplit()) {
+    ResizeArea::OnMouseCaptureLost();
+    return;
+  }
+
+  ReportResizeAmount(initial_resize_position_, true);
+}
+
 bool MultiContentsResizeArea::OnKeyPressed(const ui::KeyEvent& event) {
   int resize_amount = 0;
+  if (IsHorizontalSplit()) {
+    if (event.key_code() == ui::VKEY_UP) {
+      resize_amount = -kResizeIncrement;
+    } else if (event.key_code() == ui::VKEY_DOWN) {
+      resize_amount = kResizeIncrement;
+    } else {
+      return false;
+    }
+
+    multi_contents_view_->OnResize(resize_amount, true);
+    return true;
+  }
+
   if (event.key_code() == ui::VKEY_LEFT) {
     resize_amount = base::i18n::IsRTL() ? kResizeIncrement : -kResizeIncrement;
   } else if (event.key_code() == ui::VKEY_RIGHT) {
@@ -162,5 +273,28 @@ void MultiContentsResizeArea::SetVisible(bool visible) {
   resize_handle_->UpdateVisibility();
 }
 
+bool MultiContentsResizeArea::IsHorizontalSplit() const {
+  return split_layout_ == split_tabs::SplitTabLayout::kHorizontal;
+}
+
+bool MultiContentsResizeArea::IsResizeInProgress() {
+  return IsHorizontalSplit() ? is_resizing_horizontally_ : is_resizing();
+}
+
+void MultiContentsResizeArea::SetInitialResizePosition(int event_position) {
+  gfx::Point point(0, event_position);
+  View::ConvertPointToScreen(this, &point);
+  initial_resize_position_ = point.y();
+}
+
+void MultiContentsResizeArea::ReportResizeAmount(int resize_amount,
+                                                 bool last_update) {
+  gfx::Point point(0, resize_amount);
+  View::ConvertPointToScreen(this, &point);
+  resize_amount = point.y() - initial_resize_position_;
+  is_resizing_horizontally_ = !last_update;
+  multi_contents_view_->OnResize(resize_amount, last_update);
+}
+
 BEGIN_METADATA(MultiContentsResizeArea)
 END_METADATA
