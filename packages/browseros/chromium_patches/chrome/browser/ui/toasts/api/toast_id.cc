diff --git a/chrome/browser/ui/toasts/api/toast_id.cc b/chrome/browser/ui/toasts/api/toast_id.cc
index 81c6bc9fefaba95dcfdba84aa581dec9197f5ba8..56bc3725e0c7988df6a29e76c48048c3fd45dec0 100644
--- a/chrome/browser/ui/toasts/api/toast_id.cc
+++ b/chrome/browser/ui/toasts/api/toast_id.cc
@@ -103,6 +103,8 @@ std::string_view GetToastName(ToastId toast_id) {
       return "DictationError";
     case ToastId::kDictationStopped:
       return "DictationStopped";
+    case ToastId::kBrowserOSToast:
+      return "BrowserOSToast";
   }
 
   NOTREACHED();
