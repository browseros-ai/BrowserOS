diff --git a/chrome/browser/ui/views/frame/browser_native_widget_mac.mm b/chrome/browser/ui/views/frame/browser_native_widget_mac.mm
index 46037e75841673859243a90fc0acccf5f5da8b9b..4efda01f9b5d143dec063fb134771a54dca47ba8 100644
--- a/chrome/browser/ui/views/frame/browser_native_widget_mac.mm
+++ b/chrome/browser/ui/views/frame/browser_native_widget_mac.mm
@@ -649,6 +649,9 @@ views::Widget::InitParams BrowserNativeWidgetMac::GetWidgetParams(
     views::Widget::InitParams::Ownership ownership) {
   views::Widget::InitParams params(ownership);
   params.native_widget = this;
+  if (browser_view_) {
+    params.headless = browser_view_->browser()->is_hidden();
+  }
   return params;
 }
 
