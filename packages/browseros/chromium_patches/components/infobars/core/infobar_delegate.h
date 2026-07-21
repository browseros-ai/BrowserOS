diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 0751e0b9c6dcbdd5fac9ab4fc1f66de941fe8b90..feda40da706d9bb6c7cbeba5cbf689b6b7a75a74 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -208,6 +208,9 @@ class InfoBarDelegate {
     JS_OPTIMIZATIONS_INFOBAR_DELEGATE = 133,
     WEB_APP_BLOCKED_MIGRATION_INFOBAR_DELEGATE = 134,
     OSCRYPTASYNC_AVAILABILITY_INFOBAR_DELEGATE = 135,
+    // BrowserOS: agent installation infobar
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 136,
+    BROWSEROS_EXTENSION_INFOBAR_DELEGATE = 137,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
