diff --git a/ui/views/widget/desktop_aura/desktop_window_tree_host_win.h b/ui/views/widget/desktop_aura/desktop_window_tree_host_win.h
index 9ba64dc1b546b3a4e48e13949a555c227e02327c..a7755b89ec4c5f5ebf1c2e05cfc3373ecde465b4 100644
--- a/ui/views/widget/desktop_aura/desktop_window_tree_host_win.h
+++ b/ui/views/widget/desktop_aura/desktop_window_tree_host_win.h
@@ -368,6 +368,12 @@ class VIEWS_EXPORT DesktopWindowTreeHostWin
   // Overrides the remote session detection for testing.
   std::optional<bool> remote_session_for_testing_;
 
+  // Honors Widget::InitParams::headless: the HWND is created but never
+  // transitioned to visible via ShowWindow(SW_SHOW*), so the OS compositor
+  // (taskbar, Alt-Tab, Task View, peek preview) doesn't see it. The aura
+  // side still transitions to visible so the content compositor runs.
+  bool is_headless_ = false;
+
   // Visibility of the cursor. On Windows we can have multiple root windows and
   // the implementation of ::ShowCursor() is based on a counter, so making this
   // member static ensures that ::ShowCursor() is always called exactly once
