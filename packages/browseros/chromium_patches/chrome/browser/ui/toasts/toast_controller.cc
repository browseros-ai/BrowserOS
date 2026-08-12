diff --git a/chrome/browser/ui/toasts/toast_controller.cc b/chrome/browser/ui/toasts/toast_controller.cc
index bde46cf5ce841d428768ec3486296a15e62ef99d..b3f53943934ff5a43cba98f1b6c6ca0b09563406 100644
--- a/chrome/browser/ui/toasts/toast_controller.cc
+++ b/chrome/browser/ui/toasts/toast_controller.cc
@@ -295,8 +295,8 @@ void ToastController::ShowToast(ToastParams params) {
   const bool is_actionable =
       current_toast_spec->action_button_string_id().has_value() ||
       current_toast_spec->has_menu();
-  base::TimeDelta timeout =
-      is_actionable ? kToastWithActionTimeout : kToastDefaultTimeout;
+  base::TimeDelta timeout = params.timeout_override.value_or(
+      is_actionable ? kToastWithActionTimeout : kToastDefaultTimeout);
 
   toast_close_timer_.Start(
       FROM_HERE, timeout,
