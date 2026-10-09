diff --git a/chrome/browser/ui/toasts/api/toast_id.cc b/chrome/browser/ui/toasts/api/toast_id.cc
index 06603d62ef855e8c3bf003fb0674b5bcc31653fd..acb19cf86a86b22fb29dd5cc207ee312ec66c766 100644
--- a/chrome/browser/ui/toasts/api/toast_id.cc
+++ b/chrome/browser/ui/toasts/api/toast_id.cc
@@ -123,6 +123,8 @@ std::string_view GetToastName(ToastId toast_id) {
       return "EmailVerificationLoading";
     case ToastId::kScheduledRestartOnIdle:
       return "ScheduledRestartOnIdle";
+    case ToastId::kBrowserOSToast:
+      return "BrowserOSToast";
   }
 
   NOTREACHED();
