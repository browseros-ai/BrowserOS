diff --git a/chrome/browser/prefs/chrome_command_line_pref_store.cc b/chrome/browser/prefs/chrome_command_line_pref_store.cc
index 3e93688a11aeb52a016c1488a98d9354ec15e5ac..5f5b192e9d21151627f861c1d4a1ffe7665f976a 100644
--- a/chrome/browser/prefs/chrome_command_line_pref_store.cc
+++ b/chrome/browser/prefs/chrome_command_line_pref_store.cc
@@ -18,6 +18,7 @@
 #include "base/strings/string_split.h"
 #include "base/values.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/customization/command_line_prefs.h"
 #include "chrome/common/chrome_switches.h"
 #include "chrome/common/pref_names.h"
 #include "components/browser_sync/browser_sync_switches.h"
@@ -108,6 +109,8 @@ const CommandLinePrefStore::SwitchToPreferenceMapEntry
 ChromeCommandLinePrefStore::ChromeCommandLinePrefStore(
     const base::CommandLine* command_line)
     : CommandLinePrefStore(command_line) {
+  // BrowserOS switches that force prefs, e.g. --disable-browseros-features.
+  browseros::ApplyCommandLinePrefs(*command_line, *this);
   ApplySimpleSwitches();
   ApplyProxyMode();
   ValidateProxySwitches();
