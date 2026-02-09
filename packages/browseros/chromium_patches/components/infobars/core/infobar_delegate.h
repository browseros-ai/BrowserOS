diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 62247f8d82ca6..5bc23ffd73d95 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -212,6 +212,8 @@ class InfoBarDelegate {
     SESSION_RESTORE_INFOBAR_DELEGATE = 128,
     ROLL_BACK_MODE_B_INFOBAR_DELEGATE = 129,
     DEV_TOOLS_REMOTE_DEBUGGING_INFOBAR_DELEGATE = 130,
+    // BrowserOS: agent installation infobar
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 131,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
