diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 4f9cd9e57e3bb..862516ac3724b 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -213,6 +213,8 @@ class InfoBarDelegate {
     ROLL_BACK_MODE_B_INFOBAR_DELEGATE = 129,
     DEV_TOOLS_REMOTE_DEBUGGING_INFOBAR_DELEGATE = 130,
     STARTUP_LAUNCH_INFOBAR_DELEGATE = 131,
+    // BrowserOS: agent installation infobar
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 132,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
