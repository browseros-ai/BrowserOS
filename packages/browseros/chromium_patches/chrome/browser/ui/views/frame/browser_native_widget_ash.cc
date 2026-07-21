diff --git a/chrome/browser/ui/views/frame/browser_native_widget_ash.cc b/chrome/browser/ui/views/frame/browser_native_widget_ash.cc
index 8fea6e6e60cb5dad57f9bb977e38f5e376b9e4d2..1a8ced3ae70eb5cca594b1c66a6931816e897214 100644
--- a/chrome/browser/ui/views/frame/browser_native_widget_ash.cc
+++ b/chrome/browser/ui/views/frame/browser_native_widget_ash.cc
@@ -188,6 +188,7 @@ views::Widget::InitParams BrowserNativeWidgetAsh::GetWidgetParams(
   params.context = ash::Shell::GetPrimaryRootWindow();
 
   Browser* browser = browser_view_->browser();
+  params.headless = browser->is_hidden();
   const int32_t restore_id = browser->create_params().restore_id;
   params.init_properties_container.SetProperty(app_restore::kWindowIdKey,
                                                browser->session_id().id());
