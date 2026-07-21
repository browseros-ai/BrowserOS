diff --git a/ui/views/cocoa/native_widget_mac_ns_window_host.mm b/ui/views/cocoa/native_widget_mac_ns_window_host.mm
index c28fed2e6963c894e1f49a084d6f635fb6f20c11..55319b09cf67deaeb273a5f6d2029a55647586b1 100644
--- a/ui/views/cocoa/native_widget_mac_ns_window_host.mm
+++ b/ui/views/cocoa/native_widget_mac_ns_window_host.mm
@@ -495,6 +495,7 @@ void NativeWidgetMacNSWindowHost::InitWindow(
     window_params->is_translucent =
         params.opacity == Widget::InitParams::WindowOpacity::kTranslucent;
     window_params->is_tooltip = is_tooltip;
+    window_params->is_headless = params.headless;
 
     // macOS likes to put shadows on most things. However, frameless windows
     // (with styleMask = NSWindowStyleMaskBorderless) default to no shadow. So
