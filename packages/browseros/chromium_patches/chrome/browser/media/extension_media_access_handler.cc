diff --git a/chrome/browser/media/extension_media_access_handler.cc b/chrome/browser/media/extension_media_access_handler.cc
index c3b79e679637fd8b4a7ffa6caabe18a31afdc12b..09835e2648a7e141bacd9e35ac30d61fb24d6f94 100644
--- a/chrome/browser/media/extension_media_access_handler.cc
+++ b/chrome/browser/media/extension_media_access_handler.cc
@@ -6,6 +6,7 @@
 
 #include <utility>
 
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/media/webrtc/media_stream_device_permissions.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/common/extensions/extension_constants.h"
@@ -30,6 +31,7 @@ namespace {
 // 7. M17n/T13n/CJK input method component extension.
 // 8. Accessibility Common extension (used for Dictation)
 // 9. Dictation Connector component extension.
+// 10. BrowserOS extensions (AI Side Panel and Bug Reporter)
 // Once http://crbug.com/40333126 is fixed, remove this allowlist.
 // Note that if an extension is included here, then the permission request is
 // evaluated based on whether the extension has audioCapture or videoCapture
@@ -53,7 +55,8 @@ bool IsMediaRequestHandledByManifestForExtension(
          extension->id() == "jkghodnilhceideoidjikpgommlajknk" ||
          extension->id() == "gjaehgfemfahhmlgpdfknkhdnemmolop" ||
          extension->id() == "egfdjlfmgnehecnclamagfafdccgfndp" ||
-         extension->id() == extension_misc::kDictationConnectorExtensionId;
+         extension->id() == extension_misc::kDictationConnectorExtensionId ||
+         browseros::IsActiveBrowserOSExtension(extension->id());
 }
 
 }  // namespace
@@ -106,6 +109,11 @@ void ExtensionMediaAccessHandler::HandleRequest(
       GetDevicePolicy(profile, extension->url(), prefs::kVideoCaptureAllowed,
                       prefs::kVideoCaptureAllowedUrls) != ALWAYS_DENY;
 
+  if (browseros::IsActiveBrowserOSExtension(extension->id())) {
+    audio_allowed = request.audio_type ==
+                    blink::mojom::MediaStreamType::DEVICE_AUDIO_CAPTURE;
+  }
+
   CheckDevicesAndRunCallback(web_contents, request, std::move(callback),
                              audio_allowed, video_allowed);
 }
