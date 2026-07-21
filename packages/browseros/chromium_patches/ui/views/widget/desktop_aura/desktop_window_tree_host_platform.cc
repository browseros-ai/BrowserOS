diff --git a/ui/views/widget/desktop_aura/desktop_window_tree_host_platform.cc b/ui/views/widget/desktop_aura/desktop_window_tree_host_platform.cc
index 54dd477b3bd1f1f5bf6912bbe745b5b0ecfc66f9..b0b7dca44834d768ea634eac1356e9f5035fe73b 100644
--- a/ui/views/widget/desktop_aura/desktop_window_tree_host_platform.cc
+++ b/ui/views/widget/desktop_aura/desktop_window_tree_host_platform.cc
@@ -144,6 +144,7 @@ ui::PlatformWindowInitProperties ConvertWidgetInitParamsToInitProperties(
   properties.workspace = params.workspace;
   properties.opacity = GetPlatformWindowOpacity(params.opacity);
   properties.shadow_type = GetPlatformWindowShadowType(params.shadow_type);
+  properties.headless = params.headless;
 
   if (params.parent && params.parent->GetHost()) {
     properties.parent_widget = params.parent->GetHost()->GetAcceleratedWidget();
