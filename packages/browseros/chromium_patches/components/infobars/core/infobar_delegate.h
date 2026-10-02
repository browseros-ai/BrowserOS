diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 533b73080f66a4d68528eca6675352257bcea5eb..385bb6a60ae0229a519852cb745ab5e1f7bb42c8 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -16,7 +16,6 @@
 class ConfirmInfoBarDelegate;
 class ThemeInstalledInfoBarDelegate;
 
-
 namespace translate {
 class TranslateInfoBarDelegate;
 }
@@ -212,6 +211,9 @@ class InfoBarDelegate {
     SIGNIN_QRCODE_INFOBAR_DELEGATE = 137,
     FORMS_AI_PRIVATE_INFERENCE_INFOBAR_DELEGATE_IOS = 138,
     PASSWORD_SAVED_INFOBAR_DELEGATE_IOS = 139,
+    // BrowserOS: agent installation infobars.
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 140,
+    BROWSEROS_EXTENSION_INFOBAR_DELEGATE = 141,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
