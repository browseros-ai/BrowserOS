diff --git a/components/infobars/core/infobar_delegate.h b/components/infobars/core/infobar_delegate.h
index 20a8371d17c74..0351c5ad4b028 100644
--- a/components/infobars/core/infobar_delegate.h
+++ b/components/infobars/core/infobar_delegate.h
@@ -195,6 +195,7 @@ class InfoBarDelegate {
     PIN_INFOBAR_DELEGATE = 127,
     SESSION_RESTORE_INFOBAR_DELEGATE = 128,
     ROLL_BACK_MODE_B_INFOBAR_DELEGATE = 129,
+    BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE = 130,
   };
   // LINT.ThenChange(//tools/metrics/histograms/metadata/browser/enums.xml:InfoBarIdentifier)
 
