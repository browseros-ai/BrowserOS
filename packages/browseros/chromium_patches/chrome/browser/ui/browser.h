diff --git a/chrome/browser/ui/browser.h b/chrome/browser/ui/browser.h
index e2fcdc383237c909dfcd51a02e1a9e54cd6f7f0c..ca02328d6f98d434ea5f1c6601ea838febacf40f 100644
--- a/chrome/browser/ui/browser.h
+++ b/chrome/browser/ui/browser.h
@@ -14,6 +14,8 @@
 #include <vector>
 
 #include "base/functional/callback.h"
+#include "base/containers/flat_map.h"
+#include "base/functional/callback_helpers.h"
 #include "base/gtest_prod_util.h"
 #include "base/memory/raw_ptr.h"
 #include "base/memory/scoped_refptr.h"
@@ -266,6 +268,12 @@ class Browser : public TabStripModelObserver,
     // Whether this browser was created specifically for dragged tab(s).
     bool in_tab_dragging = false;
 
+    // Create the window as hidden (invisible to the OS compositor: no taskbar
+    // entry, no Mission Control, no Alt-Tab). Used for offscreen agent
+    // contexts. Decided at construction; does not change over a Browser's
+    // lifetime.
+    bool hidden = false;
+
     // Supply a custom BrowserWindow implementation, to be used instead of the
     // default. Intended for testing. The resulting Browser takes ownership
     // of `window`.
@@ -438,6 +446,7 @@ class Browser : public TabStripModelObserver,
   }
 
   SessionID session_id() const { return session_id_; }
+  bool is_hidden() const { return is_hidden_; }
   bool omit_from_session_restore() const { return omit_from_session_restore_; }
   bool should_trigger_session_restore() const {
     return should_trigger_session_restore_;
@@ -1114,6 +1123,19 @@ class Browser : public TabStripModelObserver,
   // restore.
   bool omit_from_session_restore_ = false;
 
+  const bool is_hidden_;
+
+  // For hidden Browsers only: ScopedClosureRunners returned by
+  // WebContents::IncrementCapturerCount, keyed by WebContents*. Holding the
+  // runner pins Visibility::kVisible so pages in the hidden window behave as
+  // foreground (unthrottled rAF, playing video, live DOM). Drop the runner
+  // (on tab detach/remove or Browser destruction) to decrement the count.
+  base::flat_map<content::WebContents*, base::ScopedClosureRunner>
+      hidden_tab_pins_;
+
+  void PinHiddenTabVisibility(content::WebContents* web_contents);
+  void UnpinHiddenTabVisibility(content::WebContents* web_contents);
+
   // If true, a new window opening should be treated like the start of a session
   // (with potential session restore, startup URLs, etc.). Otherwise, don't
   // restore the session.
