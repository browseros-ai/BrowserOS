diff --git a/chrome/browser/ui/webui/blockbrowser_first_run.h b/chrome/browser/ui/webui/blockbrowser_first_run.h
new file mode 100644
index 0000000000000..51ab05bc94583
--- /dev/null
+++ b/chrome/browser/ui/webui/blockbrowser_first_run.h
@@ -0,0 +1,354 @@
+#ifndef CHROME_BROWSER_UI_WEBUI_NXTSCAPE_FIRST_RUN_H_
+#define CHROME_BROWSER_UI_WEBUI_NXTSCAPE_FIRST_RUN_H_
+
+#include "base/memory/ref_counted_memory.h"
+#include "chrome/browser/profiles/profile.h"
+#include "content/public/browser/url_data_source.h"
+#include "content/public/browser/web_ui.h"
+#include "content/public/browser/web_ui_controller.h"
+#include "content/public/browser/webui_config.h"
+#include "services/network/public/mojom/content_security_policy.mojom.h"
+
+class UFRDataSource : public content::URLDataSource {
+ public:
+  UFRDataSource() {}
+  UFRDataSource(const UFRDataSource&) = delete;
+  UFRDataSource& operator=(const UFRDataSource&) = delete;
+
+  // URLDataSource implementation:
+  std::string GetSource() override;
+  std::string GetMimeType(const GURL& url) override;
+  std::string GetContentSecurityPolicy(network::mojom::CSPDirectiveName directive) override;
+  void StartDataRequest(const GURL& url,
+                        const content::WebContents::Getter& wc_getter,
+                        GotDataCallback callback) override;
+};
+
+// Implementation of UFRDataSource
+std::string UFRDataSource::GetSource() {
+  return "browseros-first-run";
+}
+
+std::string UFRDataSource::GetMimeType(const GURL& url) {
+  return "text/html";
+}
+
+std::string UFRDataSource::GetContentSecurityPolicy(network::mojom::CSPDirectiveName directive) {
+  if (directive == network::mojom::CSPDirectiveName::ScriptSrc)
+    return "'unsafe-inline'";
+  if (directive == network::mojom::CSPDirectiveName::StyleSrc)
+    return "'unsafe-inline' https://fonts.googleapis.com";
+  if (directive == network::mojom::CSPDirectiveName::FontSrc)
+    return "https://fonts.gstatic.com";
+  if (directive == network::mojom::CSPDirectiveName::ImgSrc)
+    return "'self' data:";
+  return std::string();
+}
+
+void UFRDataSource::StartDataRequest(const GURL& url,
+                                    const content::WebContents::Getter& wc_getter,
+                                    GotDataCallback callback) {
+  std::string source = R"(
+<!DOCTYPE html>
+<html lang="en">
+<head>
+<title>BlockBrowser — First Run</title>
+<meta charset="UTF-8">
+ <meta name="color-scheme" content="light dark">
+<style>
+ @import url(chrome://resources/css/text_defaults_md.css);
+ :root{
+  --bg: #FAF7F3;
+  --text: #1F1D1B;
+  --muted: #6B7280;
+  --border: #E8E2DA;
+  --card: #FFFFFF;
+  --accent: #FB651F;
+  --cta: #231F1A;
+  --link: #1F2937;
+  --chip-bg:#F1F4F8; --chip-border:#E2E8F0; --chip-text:#0F172A;
+  --kbd-bg:#F5F7FA; --kbd-border:#E5E7EB; --kbd-border2:#DBDEE3; --kbd-text:#111827;
+  --note-bg:#F9FAFB; --note-border:#D6D3CE; --note-text:#374151;
+  --badge-bg:#FFFFFFAA; --badge-border:var(--border); --badge-text:#4B5563;
+  --linkpill-bg:var(--card); --linkpill-text:var(--text);
+  --social-icon:#6b7280;
+  --shadow-lg: 0 24px 40px -20px rgba(0,0,0,.12);
+  --shadow-md: 0 10px 30px rgba(0,0,0,0.06);
+  --accent-purple:#a78bfa; --accent-teal:#22d3ee; --accent-amber:#f59e0b;
+  --feature-card-h: 128px;
+ }
+ /* Use Roboto everywhere */
+ html{color:var(--text); background:var(--bg); line-height:1.6; font-family: "Roboto", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, "Apple Color Emoji", "Segoe UI Emoji"; font-size: 16px;}
+ a{color:var(--link); text-decoration: none;}
+ a:hover{text-decoration: underline;}
+ h2{margin:0; font-size: 1.5rem; font-weight: 700; letter-spacing:.1px;}
+ p{margin:0;}
+ ul,ol{padding-left:1.25rem; margin: .5rem 0;}
+ code{background:var(--chip-bg); padding:.2rem .5rem; border:1px solid var(--chip-border); border-radius:.5rem; font-size:.875rem; color:var(--chip-text)}
+ kbd{font-family: ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace; background:var(--kbd-bg); border:1px solid var(--kbd-border); border-bottom-color:var(--kbd-border2); border-radius:.35rem; padding:.15rem .4rem; font-size:.85rem; color:var(--kbd-text)}
+/* Base section layout and consistent vertical rhythm */
+section{width:64rem; max-width: 92%; margin:0 auto 2.5rem; padding-top:0;}
+/* Add uniform top spacing only between subsequent sections */
+/* Default spacing between sections */
+section + section{padding-top:2.75rem;}
+/* Tighter spacing from hero to first content section */
+.hero + section{padding-top:1.25rem;}
+
+ /* Hero */
+ .hero {text-align: center; padding: 3.5rem 0 1rem 0;}
+ .hero h1 {font-family: inherit; font-size: 3.1rem; line-height:1.12; margin: 0 0 .6rem; color: var(--text); letter-spacing: .2px; font-weight:700;}
+ .hero .accent {color: var(--accent); font-style: normal; font-weight:700;}
+ .hero p {font-size: 1.05rem; color: var(--muted); margin-top:.5rem;}
+ .subtle {color:var(--muted); font-size:.95rem; margin-top:.75rem;}
+ .badge-row{display:flex; justify-content:center; gap:.5rem; margin: .75rem 0 1rem;}
+ .badge{display:inline-flex; align-items:center; gap:.4rem; padding:.25rem .55rem; font-size:.75rem; color:var(--badge-text); border:1px solid var(--badge-border); border-radius:999px; background: var(--badge-bg); backdrop-filter: saturate(120%)}
+ 
+ /* Buttons */
+.actions{margin-top:1rem}
+/* Remove stray space when no buttons are present */
+.hero .actions:empty{display:none}
+ .btn{display:inline-flex; align-items:center; gap:.5rem; padding:.8rem 1rem; border-radius:999px; border:1px solid transparent; font-weight:600; text-decoration:none}
+ .btn .icon{display:inline-flex; width:18px; height:18px}
+ .btn .icon svg{width:18px; height:18px}
+  .btn-dark{background:var(--cta); color:#fff; box-shadow: 0 8px 18px rgba(0,0,0,.18);}
+  .btn-outline{background:transparent; color:var(--text); border-color:var(--border);}
+  .btn-accent{background:var(--accent); color:#fff; border-color:transparent; box-shadow: 0 10px 20px rgba(0,0,0,.18);}
+  .btn-accent:hover{filter:brightness(1.05)}
+  .btn + .btn{margin-left:.5rem}
+
+ /* Section headers */
+.section-head{display:flex; align-items:center; justify-content:center; margin:0 0 1rem;}
+.section-head .label{font-family: inherit; font-size:1.7rem; color:var(--text); font-weight:700}
+ .section-head .label .accent{color:var(--accent); font-style: normal}
+
+ /* Light cards and groups */
+ .group {background: var(--card); border:1px solid var(--border); border-radius: 1rem; padding:1.15rem 1.25rem; box-shadow: var(--shadow-md);} 
+ .group + .group {margin-top:1rem;}
+ .title {font-size:1.15rem; color:var(--text); margin-bottom:.5rem;}
+
+ /* Legacy CTA (kept for quick link outside buttons) */
+ .cta {display:inline-block; margin-top:.75rem; padding:.85rem 1.25rem; border-radius:999px; color:#fff; background: linear-gradient(90deg,#6366f1,#8b5cf6); box-shadow:0 12px 26px rgba(99,102,241,.28); font-weight:700}
+ .cta:hover{filter:brightness(1.05);} 
+
+ /* Cards grid */
+ .grid {display:grid; grid-template-columns: repeat(auto-fit, minmax(290px, 1fr)); gap:1.25rem;}
+ .grid.features{grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap:1.5rem; row-gap:2rem; grid-auto-flow: row;}
+ @media (max-width: 720px){ 
+  .grid.features{grid-template-columns: 1fr; row-gap:1.25rem;}
+  .features .card{height:auto; padding:1rem 1.15rem}
+ }
+ .card {background:var(--card); border:1px solid var(--border); border-radius:1rem; padding:1.1rem 1.25rem; color:var(--text); box-shadow: var(--shadow-lg);} 
+ .card h3 {margin:.15rem 0 .6rem; font-size:1.06rem; color:var(--text); font-weight:700;}
+ .note {margin-top:.7rem; font-size:.92rem; color:var(--note-text); border:1px dashed var(--note-border); background:var(--note-bg); padding:.6rem .75rem; border-radius:.5rem}
+
+ /* Heading icons */
+ .hicon{display:inline-flex; align-items:center; justify-content:center; width:22px; height:22px; margin-right:.55rem; vertical-align:middle}
+ .hicon.import{color:var(--accent-purple)}
+ .hicon.key{color:var(--accent-teal)}
+ .hicon.keyboard{color:var(--accent-amber)}
+ .hicon.check{color:#02b302}
+ .hicon svg{width:22px; height:22px; fill:currentColor}
+
+ /* Banner */
+ .banner {background:var(--card); border:1px solid var(--border); border-radius:1rem; padding:1rem 1.25rem; color:var(--text); text-align:left; box-shadow: var(--shadow-lg)}
+ .banner .label{display:flex; align-items:center; font-weight:700;}
+
+/* Features cards */
+.features .card{height:var(--feature-card-h); box-shadow: var(--shadow-md); padding:.7rem 1rem}
+.features .card h3{margin:.05rem 0 .25rem; line-height:1.28}
+.features .card p{line-height:1.45}
+.features .card p + p{margin-top:.25rem !important}
+/* Icons for feature cards use accent color */
+.features .card .hicon{color: var(--accent)}
+ .muted{color:var(--muted)}
+ .examples{margin-top:.75rem}
+ .codebox{display:block; margin-top:.5rem; padding:.7rem .8rem; border-radius:.6rem; background:var(--chip-bg); border:1px solid var(--chip-border); color:var(--chip-text); font-family: ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace; font-size:.9rem}
+
+ /* Links row */
+ .links a{display:inline-flex; align-items:center; gap:.45rem; margin:.25rem .35rem; padding:.55rem .9rem; background:var(--linkpill-bg); border:1px solid var(--border); border-radius:.7rem; color:var(--linkpill-text); box-shadow: 0 12px 24px -20px rgba(0,0,0,.15)}
+ .links a:hover{background:var(--linkpill-bg); filter:brightness(1.03)}
+ .links a .icon{display:inline-flex; width:18px; height:18px; color: var(--social-icon);}
+ .links a .icon svg{width:18px; height:18px; fill: currentColor}
+
+ @media(prefers-color-scheme:dark){
+  :root{
+   --bg:#0b0f14; --text:#e5e7eb; --muted:#94a3b8; --border:#1f2a3a; --card:#101723; --cta:#161A23; --accent:#fb651f; --link:#9aa1ff;
+   --chip-bg:#0b1220; --chip-border:#1f2937; --chip-text:#e5e7eb;
+   --kbd-bg:#0b1220; --kbd-border:#1f2937; --kbd-border2:#1b2432; --kbd-text:#e5e7eb;
+   --note-bg:#0b1220; --note-border:#273144; --note-text:#cbd5e1;
+  --badge-bg:rgba(255,255,255,.08); --badge-border:#2a3342; --badge-text:#cbd5e1;
+  --linkpill-bg:#101723; --linkpill-text:#e5e7eb;
+  --social-icon:#9ca3af;
+  --shadow-lg: 0 24px 40px -20px rgba(0,0,0,.6);
+  --shadow-md: 0 10px 30px rgba(0,0,0,0.5);
+  --accent-purple:#a78bfa; --accent-teal:#22d3ee; --accent-amber:#f59e0b;
+  }
+ }
+</style>
+<base target="_blank">
+</head>
+<body>
+<section class="hero">
+ <div class="badge-row">
+  <span class="badge">Backed by YC</span>
+ </div>
+ <h1>The Open Source <span class="accent">Agentic</span> <span class="accent">Browser</span></h1>
+ <p>BlockBrowser is an AI-powered browser that lets you build and run agents to automate tedious tasks. It looks like Chrome but reimagined for the AI era.</p>
+ <div class="actions">
+  </div>
+</section>
+
+<section>
+ <div class="section-head"><span class="label">🚀 Getting Started</span></div>
+ <div style="text-align:center;">
+  <a class="btn btn-accent" href="https://bit.ly/BlockBrowser-setup">Quick start guide</a>
+  <a class="btn btn-outline" href="https://github.com/blockbrowser/blockbrowser">
+   <span class="icon" aria-hidden="true">
+    <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#eac54f"><path d="m354-287 126-76 126 77-33-144 111-96-146-13-58-136-58 135-146 13 111 97-33 143ZM233-120l65-281L80-590l288-25 112-265 112 265 288 25-218 189 65 281-247-149-247 149Zm247-350Z"/></svg>
+   </span>
+   Star us on Github
+  </a>
+  </div>
+
+ <div class="grid" style="margin-top:1rem;">
+  <div class="card">
+   <h3>
+    <span class="hicon import" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
+      <path d="M5 20h14v-2H5v2zm7-16c.55 0 1 .45 1 1v6.17l2.59-2.58 1.41 1.41L12 14.59 6.99 10l1.41-1.41L11 11.17V5c0-.55.45-1 1-1z"/>
+     </svg>
+    </span>
+    Step 1: Import your data from Chrome
+   </h3>
+   <ol>
+    <li>Navigate to <a href="chrome://settings/importData"><code>chrome://settings/importData</code></a></li>
+    <li>Click "Import"</li>
+    <li>Follow the prompts and choose "Always allow" when asked to import everything at once</li>
+   </ol>
+  </div>
+  <div class="card">
+   <h3>
+    <span class="hicon key" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" height="24" viewBox="0 -960 960 960" width="24">
+      <path d="M420-680q0-33 23.5-56.5T500-760q33 0 56.5 23.5T580-680q0 33-23.5 56.5T500-600q-33 0-56.5-23.5T420-680ZM500 0 320-180l60-80-60-80 60-85v-47q-54-32-87-86.5T260-680q0-100 70-170t170-70q100 0 170 70t70 170q0 67-33 121.5T620-472v352L500 0ZM340-680q0 56 34 98.5t86 56.5v125l-41 58 61 82-55 71 75 75 40-40v-371q52-14 86-56.5t34-98.5q0-66-47-113t-113-47q-66 0-113 47t-47 113Z"/>
+     </svg>
+    </span>
+    Step 2: BYOK (Bring Your Own Keys)
+   </h3>
+   <p class="muted">You have full control over your AI models!</p>
+   <p class="muted" style="margin-top:.35rem">Navigate to <a href="chrome://settings/browseros"><code>chrome://settings/browseros</code></a> to configure your own API keys for various providers.</p>
+   <div class="note">Note: You can even run everything locally using Ollama!</div>
+  </div>
+ </div>
+
+ <div class="banner" style="margin-top:1.1rem;">
+  <div class="label">
+   <span class="hicon check" aria-hidden="true">
+    <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#02b302"><path d="m381-240 424-424-57-56-368 367-169-170-57 57 227 226Zm0 113L42-466l169-170 170 170 366-367 172 168-538 538Z"/></svg>
+   </span>
+   Step 3: All done!
+  </div>
+  <div class="muted" style="margin-top:.4rem">Your ready to use BlockBrowser, have fun! This page can be always accessed again at <a href="chrome://browseros-first-run"><code>chrome://browseros-first-run</code></a></div>
+ </div>
+</section>
+
+<section>
+ <div class="section-head"><span class="label">✨ Key Features</span></div>
+ <div class="grid features" style="margin-top:1rem;">
+  <div class="card">
+   <h3>
+    <span class="hicon feature-globe" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#fb651f"><path d="M480-80q-82 0-155-31.5t-127.5-86Q143-252 111.5-325T80-480q0-83 31.5-155.5t86-127Q252-817 325-848.5T480-880q83 0 155.5 31.5t127 86q54.5 54.5 86 127T880-480q0 82-31.5 155t-86 127.5q-54.5 54.5-127 86T480-80Zm0-82q26-36 45-75t31-83H404q12 44 31 83t45 75Zm-104-16q-18-33-31.5-68.5T322-320H204q29 50 72.5 87t99.5 55Zm208 0q56-18 99.5-55t72.5-87H638q-9 38-22.5 73.5T584-178ZM170-400h136q-3-20-4.5-39.5T300-480q0-21 1.5-40.5T306-560H170q-5 20-7.5 39.5T160-480q0 21 2.5 40.5T170-400Zm216 0h188q3-20 4.5-39.5T580-480q0-21-1.5-40.5T574-560H386q-3 20-4.5 39.5T380-480q0 21 1.5 40.5T386-400Zm268 0h136q5-20 7.5-39.5T800-480q0-21-2.5-40.5T790-560H654q3 20 4.5 39.5T660-480q0 21-1.5 40.5T654-400Zm-16-240h118q-29-50-72.5-87T584-782q18 33 31.5 68.5T638-640Zm-234 0h152q-12-44-31-83t-45-75q-26 36-45 75t-31 83Zm-200 0h118q9-38 22.5-73.5T376-782q-56 18-99.5 55T204-640Z"/></svg>
+    </span>
+    Natural-Language Agents
+   </h3>
+   <p class="muted">Tell your browser what to do in plain English — it clicks, types, and navigates for you. No coding, no setup.</p>
+   <p class="muted">Example: "Copy these LinkedIn contacts to a Google Sheet."</p>
+  </div>
+  <div class="card">
+   <h3>
+    <span class="hicon feature-split" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#fb651f"><path d="M600-160v-80h160v-480H600v-80h160q33 0 56.5 23.5T840-720v480q0 33-23.5 56.5T760-160H600ZM440-80v-80H200q-33 0-56.5-23.5T120-240v-480q0-33 23.5-56.5T200-800h240v-80h80v800h-80Zm320-640v480-480Z"/></svg>
+    </span>
+    Split-View AI on Any Page
+   </h3>
+   <p class="muted">Open ChatGPT, Claude, or Gemini alongside any website. Get help while you work — summarize articles, draft responses, or analyze data without switching tabs.</p>
+  </div>
+  <div class="card">
+   <h3>
+    <span class="hicon feature-private" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#fb651f"><path d="M240-80q-33 0-56.5-23.5T160-160v-400q0-33 23.5-56.5T240-640h40v-80q0-83 58.5-141.5T480-920q83 0 141.5 58.5T680-720v80h40q33 0 56.5 23.5T800-560v400q0 33-23.5 56.5T720-80H240Zm0-80h480v-400H240v400Zm240-120q33 0 56.5-23.5T560-360q0-33-23.5-56.5T480-440q-33 0-56.5 23.5T400-360q0 33 23.5 56.5T480-280ZM360-640h240v-80q0-50-35-85t-85-35q-50 0-85 35t-35 85v80ZM240-160v-400 400Z"/></svg>
+    </span>
+    Private by Default
+   </h3>
+   <p class="muted">Run AI locally with Ollama or bring your own API keys. Your data never leaves your machine unless you choose.</p>
+  </div>
+  <div class="card">
+   <h3>
+    <span class="hicon feature-ea" aria-hidden="true">
+     <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="#fb651f"><path d="M360-390q-21 0-35.5-14.5T310-440q0-21 14.5-35.5T360-490q21 0 35.5 14.5T410-440q0 21-14.5 35.5T360-390Zm240 0q-21 0-35.5-14.5T550-440q0-21 14.5-35.5T600-490q21 0 35.5 14.5T650-440q0 21-14.5 35.5T600-390ZM480-160q134 0 227-93t93-227q0-24-3-46.5T786-570q-21 5-42 7.5t-44 2.5q-91 0-172-39T390-708q-32 78-91.5 135.5T160-486v6q0 134 93 227t227 93Zm0 80q-83 0-156-31.5T197-197q-54 54-85.5-127T80-480q0-83 31.5-156T197-763q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480q0 83-31.5 156T763-197q-54 54-127 85.5T480-80Zm-54-715q42 70 114 112.5T700-640q14 0 27-1.5t27-3.5q-42-70-114-112.5T480-800q-14 0-27 1.5t-27 3.5ZM177-581q51-29 89-75t57-103q-51 29-89 75t-57 103Zm249-214Zm-103 36Z"/></svg>
+    </span>
+    Your Executive Assistant
+   </h3>
+   <p class="muted">Check email, schedule meetings, draft responses — all from the sidebar. Connect Gmail, Calendar, Notion, and more with one-click MCP integration.</p>
+  </div>
+ </div>
+</section>
+
+<section>
+ <div class="section-head"><span class="label">🤝 Join our <span class="accent">community</span></span></div>
+ <p class="links" style="text-align:center; margin-top:1rem;">
+  <a href="https://discord.gg/YKwjt5vuKr">
+   <span class="icon" aria-hidden="true">
+    <svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
+     <title>Discord</title>
+     <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z"/>
+    </svg>
+   </span>
+   Discord
+  </a>
+  <a href="https://github.com/browseros-ai/BlockBrowser">
+   <span class="icon" aria-hidden="true">
+    <svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
+     <title>GitHub</title>
+     <path d="M18.906.614a3.4 3.4 0 0 0-.896.127c-.955.262-1.824.76-2.646 1.302a13.7 13.7 0 0 0-6.807-.002A11 11 0 0 0 6.71 1.016C6.136.78 5.526.585 4.899.622c-.113.01-.263.013-.315.135a5.8 5.8 0 0 0-.422 1.737 4.8 4.8 0 0 0 .223 1.923 5.4 5.4 0 0 0-1.36 3.053c-.05.453-.036.91-.015 1.365-.934-.237-1.93-.278-2.867-.035-.11.025-.174.16-.128.261.036.092.146.14.238.111.912-.232 1.881-.177 2.783.074q.045.412.107.821c-.798-.196-1.635-.093-2.42.11-.14.033-.18.238-.071.328.083.08.205.04.303.015.74-.18 1.536-.258 2.275-.027.072.248.13.5.226.741l.316.715c.321.63.806 1.167 1.365 1.596a6.8 6.8 0 0 0 1.73.942c.727.277 1.49.449 2.258.571a3.3 3.3 0 0 0-.376.498 3.2 3.2 0 0 0-.335.795c-.43.007-.865.05-1.288-.045a1.95 1.95 0 0 1-.864-.452 3.6 3.6 0 0 1-.62-.739 1 1 0  0 0-.618-.43.73.73 0 0 0-.57.11.67.67 0 0 0-.256.406c-.051.221-.011.452.059.665.122.365.351.687.626.954.485.467 1.089.799 1.726 1.006.535.17 1.112.257 1.667.143-.003.99.003 1.979-.003 2.968-.024.232-.134.45-.28.628-.1.128-.23.231-.377.301-.128.056-.268.102-.362.212a.43.43 0 0 0-.097.422.41.41 0 0 0 .265.25c.148.05.306.039.46.036.45-.013.907-.1 1.306-.318.55-.294.95-.897.925-1.532-.002-.679-.022-1.357-.032-2.036-.003-.385-.018-.771.016-1.155.028 1.009.034 2.019.054 3.028.002.324-.001.654-.09.97a1.8 1.8 0 0 1-.365.682c-.098.12-.226.22-.295.362-.06.114-.07.26.001.37.084.143.254.197.409.22.33.041.657-.073.95-.218a2.16 2.16 0 0 0 .86-.783c.209-.338.308-.736.316-1.13.01-.584.008-1.167.017-1.75.035-.297.021-.596.028-.894.005.175.003.35.004.525-.027.534-.003 1.07-.004 1.604.01.31-.013.624.049.93.062.324.195.636.398.897.234.3.551.53.902.678.294.123.63.204.943.105.156-.044.296-.18.3-.35.003-.169-.105-.312-.22-.427a1.87 1.87 0 0 1-.524-1.36V18.47c.004-.27-.012-.54.014-.809.049.405.034.814.033 1.22.004.733-.003 1.466.004 2.2.015.41.18.822.48 1.108.346.337.824.5 1.295.565q.36.05.719.026c.182-.016.37-.137.408-.324.039-.176-.059-.355-.2-.457-.093-.075-.215-.095-.318-.153-.292-.16-.5-.451-.588-.77-.045-.138-.022-.284-.028-.426.001-.89-.007-1.782.008-2.672.483.097.983.043 1.455-.084a4.6 4.6 0 0 0 1.794-.94c.334-.29.618-.651.761-1.073.068-.205.11-.426.068-.641a.7.7 0 0 0-.233-.414.72.72 0 0 0-.583-.137 1 1 0  0 0-.632.431c-.206.314-.445.612-.744.841a1.9 1.9 0 0 1-.718.346c-.43.103-.876.059-1.314.051a3.9 3.9 0 0 0-.72-1.269c-.032-.022.018-.042.036-.041 1.228-.18 2.453-.528 3.514-1.188a5.15 5.15 0 0 0 1.638-1.593 3 3 0 0 0 .21-.383c.096-.214.203-.423.285-.644a8 8 0  0 0 .218-.725c.661-.228 1.38-.193 2.057-.057q.187.037.375.084c.07.017.154.017.21-.036.104-.084.073-.278-.054-.323-.81-.218-1.677-.314-2.5-.104a9 9 0 0 0 .109-.792c0-.031.038-.032.06-.041.903-.252 1.874-.306 2.788-.073.12.038.227-.057.263-.165v-.037c-.031-.073-.074-.156-.16-.173-.956-.246-1.971-.197-2.92.052.038-.68.033-1.37-.125-2.037a5.5 5.5 0 0 0-1.251-2.399c.386-1.133.283-2.39-.134-3.499-.04-.105-.07-.249-.204-.27q-.15-.03-.3-.032"/>
+    </svg>
+   </span>
+   GitHub
+  </a>
+  <a href="https://x.com/blockbrowser_ai">
+   <span class="icon" aria-hidden="true">
+    <svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
+     <title>X</title>
+     <path d="M18.901 1.153h3.68l-8.04 9.19L24 22.846h-7.406l-5.8-7.584-6.638 7.584H.474l8.6-9.83L0 1.154h7.594l5.243 6.932ZM17.61 20.644h2.039L6.486 3.24H4.298Z"/>
+    </svg>
+   </span>
+   Twitter
+  </a>
+ </p>
+ <p class="subtle" style="text-align:center;">Have questions or want to contribute? We’d love to hear from you.</p>
+</section>
+
+<!-- No script needed for this static page -->
+</body>
+</html>
+  )";
+  std::move(callback).Run(base::MakeRefCounted<base::RefCountedString>(std::move(source)));
+}
+
+class BlockBrowserFirstRun;
+class BlockBrowserFirstRunUIConfig : public content::DefaultWebUIConfig<BlockBrowserFirstRun> {
+  public:
+   BlockBrowserFirstRunUIConfig() : DefaultWebUIConfig("chrome", "browseros-first-run") {}
+};
+
+class BlockBrowserFirstRun : public content::WebUIController {
+ public:
+  BlockBrowserFirstRun(content::WebUI* web_ui) : content::WebUIController(web_ui) {
+    content::URLDataSource::Add(Profile::FromWebUI(web_ui), std::make_unique<UFRDataSource>());
+  }
+  BlockBrowserFirstRun(const BlockBrowserFirstRun&) = delete;
+  BlockBrowserFirstRun& operator=(const BlockBrowserFirstRun&) = delete;
+};
+
+#endif  // CHROME_BROWSER_UI_WEBUI_NXTSCAPE_FIRST_RUN_H_
