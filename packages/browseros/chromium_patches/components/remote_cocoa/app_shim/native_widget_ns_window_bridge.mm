diff --git a/components/remote_cocoa/app_shim/native_widget_ns_window_bridge.mm b/components/remote_cocoa/app_shim/native_widget_ns_window_bridge.mm
index 955e295f85213453829529bb56c941901fd08ee6..43072b90d3f4fe286d5fa16e4eb18ba504b0c921 100644
--- a/components/remote_cocoa/app_shim/native_widget_ns_window_bridge.mm
+++ b/components/remote_cocoa/app_shim/native_widget_ns_window_bridge.mm
@@ -569,7 +569,7 @@ void NativeWidgetNSWindowBridge::InitWindow(
   is_translucent_window_ = params->is_translucent;
   pending_restoration_data_ = params->state_restoration_data.Clone();
 
-  if (display::Screen::Get()->IsHeadless()) {
+  if (params->is_headless || display::Screen::Get()->IsHeadless()) {
     [window_ setIsHeadless:YES];
   }
 
