diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 0751e0b9c6dcbdd5fac9ab4fc1f66de941fe8b90..af49686dd5feb4367ceeede6ec0d1c07cfcfa7c6 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -16,7 +16,6 @@
 class ConfirmInfoBarDelegate;
 class ThemeInstalledInfoBarDelegate;
 
-
 namespace translate {
 class TranslateInfoBarDelegate;
 }
@@ -208,6 +207,9 @@ class InfoBarDelegate {
     JS_OPTIMIZATIONS_INFOBAR_DELEGATE = 133,
     WEB_APP_BLOCKED_MIGRATION_INFOBAR_DELEGATE = 134,
     OSCRYPTASYNC_AVAILABILITY_INFOBAR_DELEGATE = 135,
+    // BrowserOS: agent installation infobars.
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 136,
+    BROWSEROS_EXTENSION_INFOBAR_DELEGATE = 137,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
