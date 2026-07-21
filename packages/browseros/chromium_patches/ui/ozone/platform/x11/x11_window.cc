diff --git a/ui/ozone/platform/x11/x11_window.cc b/ui/ozone/platform/x11/x11_window.cc
index b0b4ea578e18be99935fe966b04492039f0a21bd..adb433ffefda79ae49186106bbf89c73004e27b6 100644
--- a/ui/ozone/platform/x11/x11_window.cc
+++ b/ui/ozone/platform/x11/x11_window.cc
@@ -370,6 +370,14 @@ void X11Window::Initialize(PlatformWindowInitProperties properties) {
     window_properties_.insert(x11::GetAtom("_NET_WM_STATE_SKIP_TASKBAR"));
   }
 
+  // Headless windows are agent-owned — never surface in taskbar, pager, or
+  // window-switcher even if a WM ignores the unmap (see Show()).
+  is_headless_ = properties.headless;
+  if (is_headless_) {
+    window_properties_.insert(x11::GetAtom("_NET_WM_STATE_SKIP_TASKBAR"));
+    window_properties_.insert(x11::GetAtom("_NET_WM_STATE_SKIP_PAGER"));
+  }
+
   // If the window should stay on top of other windows, add the
   // _NET_WM_STATE_ABOVE property.
   is_always_on_top_ = properties.keep_on_top;
@@ -495,6 +503,15 @@ void X11Window::Show(bool inactive) {
     return;
   }
 
+  if (is_headless_) {
+    // Headless: the XWindow stays unmapped so no WM sees it. Notify the
+    // delegate as if we mapped so the content compositor runs and paints.
+    window_mapped_in_client_ = true;
+    platform_window_delegate_->OnWindowStateChanged(
+        PlatformWindowState::kUnknown, PlatformWindowState::kNormal);
+    return;
+  }
+
   Map(inactive);
 }
 
