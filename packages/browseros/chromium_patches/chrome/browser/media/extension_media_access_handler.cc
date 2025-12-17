diff --git a/chrome/browser/media/extension_media_access_handler.cc b/chrome/browser/media/extension_media_access_handler.cc
index 8946078143534..69894bea5d27c 100644
--- a/chrome/browser/media/extension_media_access_handler.cc
+++ b/chrome/browser/media/extension_media_access_handler.cc
@@ -6,6 +6,7 @@
 
 #include <utility>
 
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/media/webrtc/media_stream_device_permissions.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/common/extensions/extension_constants.h"
@@ -28,6 +29,7 @@ namespace {
 // 6. XKB input method component extension.
 // 7. M17n/T13n/CJK input method component extension.
 // 8. Accessibility Common extension (used for Dictation)
+// 9. BrowserOS extensions (AI Side Panel and Bug Reporter)
 // Once http://crbug.com/292856 is fixed, remove this allowlist.
 bool IsMediaRequestAllowedForExtension(const extensions::Extension* extension) {
   return extension->id() == extension_misc::kKeyboardExtensionId ||
@@ -37,7 +39,9 @@ bool IsMediaRequestAllowedForExtension(const extensions::Extension* extension) {
          extension->id() == "nbpagnldghgfoolbancepceaanlmhfmd" ||
          extension->id() == "jkghodnilhceideoidjikpgommlajknk" ||
          extension->id() == "gjaehgfemfahhmlgpdfknkhdnemmolop" ||
-         extension->id() == "egfdjlfmgnehecnclamagfafdccgfndp";
+         extension->id() == "egfdjlfmgnehecnclamagfafdccgfndp" ||
+         // BrowserOS extensions
+         browseros::IsBrowserOSExtension(extension->id());
 }
 
 }  // namespace
@@ -90,6 +94,12 @@ void ExtensionMediaAccessHandler::HandleRequest(
       GetDevicePolicy(profile, extension->url(), prefs::kVideoCaptureAllowed,
                       prefs::kVideoCaptureAllowedUrls) != ALWAYS_DENY;
 
+  // For BrowserOS extensions in sidepanel, allow audio for teach mode
+  if (browseros::IsBrowserOSExtension(extension->id())) {
+    audio_allowed = request.audio_type ==
+                    blink::mojom::MediaStreamType::DEVICE_AUDIO_CAPTURE;
+  }
+
   CheckDevicesAndRunCallback(web_contents, request, std::move(callback),
                              audio_allowed, video_allowed);
 }
