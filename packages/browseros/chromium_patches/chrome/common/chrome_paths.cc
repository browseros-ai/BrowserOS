diff --git a/chrome/common/chrome_paths.cc b/chrome/common/chrome_paths.cc
index 56ff14c59c12ac006dbb6a68fe2d430dee6eafd7..ebc010833628dd76a5e43bfa1926886d7fe2a3aa 100644
--- a/chrome/common/chrome_paths.cc
+++ b/chrome/common/chrome_paths.cc
@@ -506,6 +506,19 @@ bool PathProvider(int key, base::FilePath* result) {
       create_dir = true;
       break;
 
+    case chrome::DIR_BROWSEROS_BUNDLED_EXTENSIONS:
+#if BUILDFLAG(IS_MAC)
+      cur = base::apple::FrameworkBundlePath();
+      cur = cur.Append(FILE_PATH_LITERAL("Resources"))
+                .Append(FILE_PATH_LITERAL("browseros_extensions"));
+#else
+      if (!base::PathService::Get(base::DIR_MODULE, &cur)) {
+        return false;
+      }
+      cur = cur.Append(FILE_PATH_LITERAL("browseros_extensions"));
+#endif
+      break;
+
     default:
       return false;
   }
