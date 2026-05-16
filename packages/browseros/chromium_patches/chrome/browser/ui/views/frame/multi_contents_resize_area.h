diff --git a/chrome/browser/ui/views/frame/multi_contents_resize_area.h b/chrome/browser/ui/views/frame/multi_contents_resize_area.h
index c34534ee850b1..1d782e658c6ed 100644
--- a/chrome/browser/ui/views/frame/multi_contents_resize_area.h
+++ b/chrome/browser/ui/views/frame/multi_contents_resize_area.h
@@ -6,6 +6,8 @@
 #define CHROME_BROWSER_UI_VIEWS_FRAME_MULTI_CONTENTS_RESIZE_AREA_H_
 
 #include "base/memory/raw_ptr.h"
+#include "components/split_tabs/split_tab_visual_data.h"
+#include "ui/base/cursor/cursor.h"
 #include "ui/base/interaction/element_identifier.h"
 #include "ui/views/controls/resize_area.h"
 #include "ui/views/focus/focus_manager.h"
@@ -24,6 +26,7 @@ class MultiContentsResizeHandle : public views::View,
 
   MultiContentsResizeHandle();
 
+  void SetSplitLayout(split_tabs::SplitTabLayout split_layout);
   void UpdateVisibility();
 
   // views::View:
@@ -44,15 +47,31 @@ class MultiContentsResizeArea : public views::ResizeArea {
 
   explicit MultiContentsResizeArea(MultiContentsView* multi_contents_view);
 
+  void SetSplitLayout(split_tabs::SplitTabLayout split_layout);
+
   // views::ResizeArea:
+  ui::Cursor GetCursor(const ui::MouseEvent& event) override;
   void OnGestureEvent(ui::GestureEvent* event) override;
+  bool OnMousePressed(const ui::MouseEvent& event) override;
+  bool OnMouseDragged(const ui::MouseEvent& event) override;
   void OnMouseReleased(const ui::MouseEvent& event) override;
+  void OnMouseCaptureLost() override;
   bool OnKeyPressed(const ui::KeyEvent& event) override;
   void OnMouseMoved(const ui::MouseEvent& event) override;
   void OnMouseExited(const ui::MouseEvent& event) override;
   void SetVisible(bool visible) override;
 
  private:
+  bool IsHorizontalSplit() const;
+  bool IsResizeInProgress();
+  void SetInitialResizePosition(int event_position);
+  void ReportResizeAmount(int resize_amount, bool last_update);
+
+  split_tabs::SplitTabLayout split_layout_ =
+      split_tabs::SplitTabLayout::kVertical;
+  bool is_resizing_horizontally_ = false;
+  int initial_resize_position_ = 0;
+
   raw_ptr<MultiContentsView> multi_contents_view_;
   raw_ptr<MultiContentsResizeHandle> resize_handle_;
 };
