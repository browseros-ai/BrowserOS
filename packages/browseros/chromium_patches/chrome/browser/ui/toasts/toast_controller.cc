diff --git a/chrome/browser/ui/toasts/toast_controller.cc b/chrome/browser/ui/toasts/toast_controller.cc
index fe034ae01f3d7f44675a10d52c081197f89edda3..6d3a077f0b33a12ab41824bb44f781b2dd219bf9 100644
--- a/chrome/browser/ui/toasts/toast_controller.cc
+++ b/chrome/browser/ui/toasts/toast_controller.cc
@@ -294,8 +294,8 @@ void ToastController::ShowToast(ToastParams params) {
   const bool is_actionable =
       current_toast_spec->action_button_string_id().has_value() ||
       current_toast_spec->has_menu();
-  base::TimeDelta timeout =
-      is_actionable ? kToastWithActionTimeout : kToastDefaultTimeout;
+  base::TimeDelta timeout = params.timeout_override.value_or(
+      is_actionable ? kToastWithActionTimeout : kToastDefaultTimeout);
 
   toast_close_timer_.Start(
       FROM_HERE, timeout,
