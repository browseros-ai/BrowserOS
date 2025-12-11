diff --git a/chrome/browser/ui/views/frame/browser_desktop_window_tree_host_win.cc b/chrome/browser/ui/views/frame/browser_desktop_window_tree_host_win.cc
index 0f38653c471db..6efb465537746 100644
--- a/chrome/browser/ui/views/frame/browser_desktop_window_tree_host_win.cc
+++ b/chrome/browser/ui/views/frame/browser_desktop_window_tree_host_win.cc
@@ -609,6 +609,9 @@ SkBitmap GetBadgedIconBitmapForProfile(Profile* profile) {
 }
 
 void BrowserDesktopWindowTreeHostWin::SetWindowIcon(bool badged) {
+  // Always use unbadged app icon - badged icons require profile icon
+  // infrastructure that may not be available.
+  badged = false;
   // Hold onto the previous icon so that the currently displayed
   // icon is valid until replaced with the new icon.
   base::win::ScopedGDIObject<HICON> previous_icon = std::move(icon_handle_);
