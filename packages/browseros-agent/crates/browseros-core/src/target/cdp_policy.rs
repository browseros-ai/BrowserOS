//! Explicit method semantics for bound CDP. The parameter tables are selected from the pinned
//! native protocol, not inferred by scanning JSON field names or allowing entire domains.
use super::*;

pub(super) fn unsupported(method: &str) -> CoreError {
    TargetError::CdpUnsupported {
        method: method.into(),
    }
    .into()
}

pub(super) fn page_method(method: &str, params: &Value) -> Result<(), CoreError> {
    let allowed = page_parameters(method).ok_or_else(|| unsupported(method))?;
    validate_parameters(method, params, allowed)
}
fn validate_parameters(method: &str, params: &Value, allowed: &[&str]) -> Result<(), CoreError> {
    let object = params.as_object().ok_or_else(|| unsupported(method))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(unsupported(method));
    }
    Ok(())
}

fn page_parameters(method: &str) -> Option<&'static [&'static str]> {
    match method {
        "Autofill.getCredentialContext" => Some(&[]),
        "Autofill.listCredentials" => {
            Some(&["operationId", "documentId", "query", "limit", "cursor"])
        }
        "Autofill.addCredential" => Some(&["operationId", "documentId", "username", "password"]),
        "Autofill.fillCredential" => Some(&[
            "operationId",
            "documentId",
            "credentialId",
            "username",
            "password",
        ]),
        "Autofill.cancelCredentialOperation" => Some(&["operationId"]),
        "Runtime.enable" => Some(&[]),
        "Runtime.disable" => Some(&[]),
        "Runtime.evaluate" => Some(&[
            "expression",
            "objectGroup",
            "includeCommandLineAPI",
            "silent",
            "contextId",
            "returnByValue",
            "generatePreview",
            "userGesture",
            "awaitPromise",
            "throwOnSideEffect",
            "timeout",
            "disableBreaks",
            "replMode",
            "allowUnsafeEvalBlockedByCSP",
            "uniqueContextId",
            "serializationOptions",
        ]),
        "Runtime.callFunctionOn" => Some(&[
            "functionDeclaration",
            "objectId",
            "arguments",
            "silent",
            "returnByValue",
            "generatePreview",
            "userGesture",
            "awaitPromise",
            "executionContextId",
            "objectGroup",
            "throwOnSideEffect",
            "uniqueContextId",
            "serializationOptions",
        ]),
        "Runtime.getProperties" => Some(&[
            "objectId",
            "ownProperties",
            "accessorPropertiesOnly",
            "generatePreview",
            "nonIndexedPropertiesOnly",
        ]),
        "Runtime.releaseObject" => Some(&["objectId"]),
        "Runtime.releaseObjectGroup" => Some(&["objectGroup"]),
        "Runtime.runIfWaitingForDebugger" => Some(&[]),
        "Runtime.discardConsoleEntries" => Some(&[]),
        "Runtime.awaitPromise" => Some(&["promiseObjectId", "returnByValue", "generatePreview"]),
        "Runtime.compileScript" => Some(&[
            "expression",
            "sourceURL",
            "persistScript",
            "executionContextId",
        ]),
        "Runtime.runScript" => Some(&[
            "scriptId",
            "executionContextId",
            "objectGroup",
            "silent",
            "includeCommandLineAPI",
            "returnByValue",
            "generatePreview",
            "awaitPromise",
        ]),
        "Runtime.queryObjects" => Some(&["prototypeObjectId", "objectGroup"]),
        "Runtime.addBinding" => Some(&["name", "executionContextId", "executionContextName"]),
        "Runtime.removeBinding" => Some(&["name"]),
        "Runtime.getIsolateId" => Some(&[]),
        "Runtime.getHeapUsage" => Some(&[]),
        "DOM.enable" => Some(&["includeWhitespace"]),
        "DOM.disable" => Some(&[]),
        "DOM.getDocument" => Some(&["depth", "pierce"]),
        "DOM.getFlattenedDocument" => Some(&["depth", "pierce"]),
        "DOM.describeNode" => Some(&["nodeId", "backendNodeId", "objectId", "depth", "pierce"]),
        "DOM.resolveNode" => Some(&[
            "nodeId",
            "backendNodeId",
            "objectGroup",
            "executionContextId",
        ]),
        "DOM.requestNode" => Some(&["objectId"]),
        "DOM.querySelector" => Some(&["nodeId", "selector"]),
        "DOM.querySelectorAll" => Some(&["nodeId", "selector"]),
        "DOM.performSearch" => Some(&["query", "includeUserAgentShadowDOM"]),
        "DOM.getSearchResults" => Some(&["searchId", "fromIndex", "toIndex"]),
        "DOM.discardSearchResults" => Some(&["searchId"]),
        "DOM.collectClassNamesFromSubtree" => Some(&["nodeId"]),
        "DOM.getAttributes" => Some(&["nodeId"]),
        "DOM.getBoxModel" => Some(&["nodeId", "backendNodeId", "objectId"]),
        "DOM.getContentQuads" => Some(&["nodeId", "backendNodeId", "objectId"]),
        "DOM.getOuterHTML" => Some(&["nodeId", "backendNodeId", "objectId", "includeShadowDOM"]),
        "DOM.getNodeForLocation" => Some(&[
            "x",
            "y",
            "includeUserAgentShadowDOM",
            "ignorePointerEventsNone",
        ]),
        "DOM.getFrameOwner" => Some(&["frameId"]),
        "DOM.pushNodesByBackendIdsToFrontend" => Some(&["backendNodeIds"]),
        "DOM.pushNodeByPathToFrontend" => Some(&["path"]),
        "DOM.scrollIntoViewIfNeeded" => Some(&["nodeId", "backendNodeId", "objectId", "rect"]),
        "DOM.focus" => Some(&["nodeId", "backendNodeId", "objectId"]),
        "DOM.setFileInputFiles" => Some(&["files", "nodeId", "backendNodeId", "objectId"]),
        "Page.enable" => Some(&["enableFileChooserOpenedEvent"]),
        "Page.disable" => Some(&[]),
        "Page.navigate" => Some(&[
            "url",
            "referrer",
            "transitionType",
            "frameId",
            "referrerPolicy",
        ]),
        "Page.reload" => Some(&["ignoreCache", "scriptToEvaluateOnLoad", "loaderId"]),
        "Page.getNavigationHistory" => Some(&[]),
        "Page.navigateToHistoryEntry" => Some(&["entryId"]),
        "Page.getFrameTree" => Some(&[]),
        "Page.getLayoutMetrics" => Some(&[]),
        "Page.captureScreenshot" => Some(&[
            "format",
            "quality",
            "clip",
            "fromSurface",
            "captureBeyondViewport",
            "optimizeForSpeed",
        ]),
        "Page.captureSnapshot" => Some(&["format"]),
        "Page.createIsolatedWorld" => Some(&[
            "frameId",
            "worldName",
            "grantUniveralAccess",
            "contentSecurityPolicy",
        ]),
        "Page.handleJavaScriptDialog" => Some(&["accept", "promptText"]),
        "Page.stopLoading" => Some(&[]),
        "Page.setDocumentContent" => Some(&["frameId", "html"]),
        "Page.bringToFront" => Some(&[]),
        "Page.getResourceTree" => Some(&[]),
        "Page.getResourceContent" => Some(&["frameId", "url"]),
        "Page.getAppManifest" => Some(&["manifestId"]),
        "Page.printToPDF" => Some(&[
            "landscape",
            "displayHeaderFooter",
            "printBackground",
            "scale",
            "paperWidth",
            "paperHeight",
            "marginTop",
            "marginBottom",
            "marginLeft",
            "marginRight",
            "pageRanges",
            "headerTemplate",
            "footerTemplate",
            "preferCSSPageSize",
            "transferMode",
            "generateTaggedPDF",
            "generateDocumentOutline",
        ]),
        "Input.dispatchKeyEvent" => Some(&[
            "type",
            "modifiers",
            "timestamp",
            "text",
            "unmodifiedText",
            "keyIdentifier",
            "code",
            "key",
            "windowsVirtualKeyCode",
            "nativeVirtualKeyCode",
            "autoRepeat",
            "isKeypad",
            "isSystemKey",
            "location",
            "commands",
        ]),
        "Input.dispatchMouseEvent" => Some(&[
            "type",
            "x",
            "y",
            "modifiers",
            "timestamp",
            "button",
            "buttons",
            "clickCount",
            "force",
            "tangentialPressure",
            "tiltX",
            "tiltY",
            "twist",
            "deltaX",
            "deltaY",
            "pointerType",
        ]),
        "Input.dispatchTouchEvent" => Some(&["type", "touchPoints", "modifiers", "timestamp"]),
        "Input.insertText" => Some(&["text"]),
        "Input.synthesizeScrollGesture" => Some(&[
            "x",
            "y",
            "xDistance",
            "yDistance",
            "xOverscroll",
            "yOverscroll",
            "preventFling",
            "speed",
            "gestureSourceType",
            "repeatCount",
            "repeatDelayMs",
            "interactionMarkerName",
        ]),
        "Input.synthesizePinchGesture" => Some(&[
            "x",
            "y",
            "scaleFactor",
            "relativeSpeed",
            "gestureSourceType",
        ]),
        "Input.synthesizeTapGesture" => {
            Some(&["x", "y", "duration", "tapCount", "gestureSourceType"])
        }
        "Input.setIgnoreInputEvents" => Some(&["ignore"]),
        "Input.emulateTouchFromMouseEvent" => Some(&[
            "type",
            "x",
            "y",
            "button",
            "timestamp",
            "deltaX",
            "deltaY",
            "modifiers",
            "clickCount",
        ]),
        "Accessibility.enable" => Some(&[]),
        "Accessibility.disable" => Some(&[]),
        "Accessibility.getFullAXTree" => Some(&["depth", "frameId"]),
        "Accessibility.getPartialAXTree" => {
            Some(&["nodeId", "backendNodeId", "objectId", "fetchRelatives"])
        }
        "Accessibility.getRootAXNode" => Some(&["frameId"]),
        "Accessibility.getAXNodeAndAncestors" => Some(&["nodeId", "backendNodeId", "objectId"]),
        "Accessibility.getChildAXNodes" => Some(&["id", "frameId"]),
        "Accessibility.queryAXTree" => Some(&[
            "nodeId",
            "backendNodeId",
            "objectId",
            "accessibleName",
            "role",
        ]),
        "Network.enable" => Some(&[
            "maxTotalBufferSize",
            "maxResourceBufferSize",
            "maxPostDataSize",
            "reportDirectSocketTraffic",
            "enableDurableMessages",
        ]),
        "Network.disable" => Some(&[]),
        "Network.getCookies" => Some(&["urls"]),
        "Network.getResponseBody" => Some(&["requestId"]),
        "Network.getRequestPostData" => Some(&["requestId"]),
        "Network.searchInResponseBody" => Some(&["requestId", "query", "caseSensitive", "isRegex"]),
        "Network.getSecurityIsolationStatus" => Some(&["frameId"]),
        "Emulation.setFocusEmulationEnabled" => Some(&["enabled"]),
        "Emulation.setDeviceMetricsOverride" => Some(&[
            "width",
            "height",
            "deviceScaleFactor",
            "mobile",
            "scale",
            "screenWidth",
            "screenHeight",
            "positionX",
            "positionY",
            "dontSetVisibleSize",
            "screenOrientation",
            "viewport",
            "displayFeature",
            "devicePosture",
            "scrollbarType",
            "screenOrientationLockEmulation",
            "viewportMeta",
        ]),
        "Emulation.clearDeviceMetricsOverride" => Some(&[]),
        "Emulation.setTouchEmulationEnabled" => Some(&["enabled", "maxTouchPoints"]),
        "Emulation.setEmulatedMedia" => Some(&["media", "features"]),
        _ => None,
    }
}

/// Native tab methods require exactly one selector. Caller aliases are resolved together
/// during admission, then translated to the admitted target incarnation for publication.
fn use_target_selector(method: &str, params: &mut Value, page: &PageInfo) -> Result<(), CoreError> {
    let params = params.as_object_mut().ok_or_else(|| unsupported(method))?;
    params.remove("tabId");
    params.insert("targetId".into(), json!(page.target_id.as_str()));
    Ok(())
}

fn root_parameters(method: &str) -> Option<&'static [&'static str]> {
    match method {
        "Browser.getVersion" => Some(&[]),
        "Browser.getWindows" => Some(&[]),
        "Browser.getTabs" => Some(&["windowId", "includeHidden"]),
        "Browser.getTabGroups" => Some(&["windowId"]),
        "Browser.getTabInfo" => Some(&["targetId", "tabId"]),
        "Browser.closeTab" => Some(&["targetId", "tabId"]),
        "Browser.activateTab" => Some(&["targetId", "tabId"]),
        "Browser.showTab" => Some(&["targetId", "tabId", "windowId", "index", "activate"]),
        "Browser.getActiveTab" => Some(&["windowId"]),
        "Browser.moveTab" => Some(&["targetId", "tabId", "windowId", "index"]),
        "Browser.createTab" => Some(&[
            "url",
            "windowId",
            "index",
            "background",
            "pinned",
            "browserContextId",
        ]),
        "Browser.createWindow" => {
            Some(&["url", "bounds", "windowType", "hidden", "browserContextId"])
        }
        "Browser.closeWindow" => Some(&["windowId"]),
        "Browser.activateWindow" => Some(&["windowId"]),
        "Browser.setWindowVisibility" => Some(&["windowId", "visible", "activate"]),
        "Browser.createTabGroup" => Some(&["tabIds", "title"]),
        "Browser.addTabsToGroup" => Some(&["groupId", "tabIds"]),
        "Browser.removeTabsFromGroup" => Some(&["tabIds"]),
        "Browser.updateTabGroup" => Some(&["groupId", "title", "color", "collapsed"]),
        "Browser.closeTabGroup" => Some(&["groupId"]),
        "Browser.moveTabGroup" => Some(&["groupId", "windowId", "index"]),
        "Browser.setPermission" => Some(&[
            "permission",
            "setting",
            "origin",
            "embeddedOrigin",
            "browserContextId",
        ]),
        "Browser.grantPermissions" => Some(&["permissions", "origin", "browserContextId"]),
        "Browser.resetPermissions" => Some(&["browserContextId"]),
        "Browser.setDownloadBehavior" => Some(&[
            "behavior",
            "browserContextId",
            "downloadPath",
            "eventsEnabled",
        ]),
        "History.search" => Some(&[
            "query",
            "maxResults",
            "startTime",
            "endTime",
            "browserContextId",
        ]),
        "History.getRecent" => Some(&["maxResults", "browserContextId"]),
        "History.deleteUrl" => Some(&["url", "browserContextId"]),
        "History.deleteRange" => Some(&["startTime", "endTime", "browserContextId"]),
        "Storage.getCookies" => Some(&["browserContextId"]),
        "Storage.setCookies" => Some(&["cookies", "browserContextId"]),
        "Storage.clearCookies" => Some(&["browserContextId"]),
        "Target.getTargets" => Some(&["filter"]),
        "Target.createTarget" => Some(&[
            "url",
            "left",
            "top",
            "width",
            "height",
            "windowState",
            "browserContextId",
            "enableBeginFrameControl",
            "newWindow",
            "background",
            "forTab",
            "hidden",
            "focus",
        ]),
        _ => None,
    }
}

/// Entities admitted while preparing one native request. Recheck them after all async lookups,
/// so a tab drag or group edit during preparation cannot authorize a different mutation set.
#[derive(Default)]
struct DispatchAdmission {
    tabs: Vec<PageInfo>,
    groups: Vec<Value>,
    windows: Vec<WindowId>,
    group_window: Option<WindowId>,
}

impl BrowserView {
    async fn admit_dispatch(&self, admission: &DispatchAdmission) -> Result<(), CoreError> {
        let live = self
            .validate_live()
            .await?
            .ok_or_else(|| unsupported("native_dispatch"))?;
        for id in &admission.windows {
            if !live
                .windows
                .iter()
                .any(|window| window.info.window_id == id.0 && live.scope.window_matches(window))
            {
                return Err(TargetError::WindowOutOfScope(*id).into());
            }
        }
        let mut checked = BTreeSet::new();
        for expected in &admission.tabs {
            if !checked.insert(expected.page_id.clone()) {
                continue;
            }
            let fresh = self
                .session
                .pages
                .refresh_with_epoch(expected.page_id.clone(), self.epoch_guard()?)
                .await?
                .ok_or_else(|| CoreError::UnknownPage(expected.page_id.clone()))?;
            live.scope.admit_page(&fresh, &live.windows)?;
            if admission
                .group_window
                .is_some_and(|window| fresh.window_id != Some(window))
            {
                return Err(TargetError::PageOutOfScope(fresh.page_id).into());
            }
            if fresh.target_id != expected.target_id || fresh.tab_id != expected.tab_id {
                return Err(TargetError::PageTargetChanged(expected.page_id.clone()).into());
            }
            for group in &admission.groups {
                if group["tabIds"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| *id == fresh.tab_id.0))
                    && (fresh.window_id.map(|window| window.0) != group["windowId"].as_i64()
                        || fresh.group_id.as_deref() != group["groupId"].as_str())
                {
                    return Err(TargetError::GroupOutOfScope(
                        group["groupId"].as_str().unwrap_or_default().into(),
                    )
                    .into());
                }
            }
        }
        // Group operations can affect every member. Re-read the member set after the final
        // tab handoff, otherwise a concurrent group drag/edit could expand or relocate it.
        if !admission.groups.is_empty() {
            let current = self
                .send_at_capture("Browser.getTabGroups", json!({}), None)
                .await?;
            for expected in &admission.groups {
                let id = expected["groupId"].as_str().unwrap_or_default();
                let unchanged = current["groups"].as_array().is_some_and(|groups| {
                    groups.iter().any(|group| {
                        group["groupId"] == id
                            && group["windowId"] == expected["windowId"]
                            && group["tabIds"] == expected["tabIds"]
                    })
                });
                if !unchanged {
                    return Err(TargetError::GroupOutOfScope(id.into()).into());
                }
            }
        }
        // Creation callbacks keep their original target even when a helper re-resolved PageId
        // during the lookups above. The transport closes the remaining connection-epoch race;
        // native IDs still do not provide an atomic expected-window assertion for tab drags.
        self.check_epoch(&live.scope)?;
        self.validate_expected_created_page().await
    }

    async fn native_tab(&self, method: &str, params: &Value) -> Result<PageInfo, CoreError> {
        let tab = params
            .get("tabId")
            .map(|id| id.as_i64().map(TabId).ok_or_else(|| unsupported(method)))
            .transpose()?;
        let target = params
            .get("targetId")
            .map(|id| id.as_str().ok_or_else(|| unsupported(method)))
            .transpose()?;
        if tab.is_none() && target.is_none() {
            return Err(unsupported(method));
        }
        let pages = self
            .session
            .pages
            .list_with_epoch(self.epoch_guard()?)
            .await?;
        let selected = pages
            .into_iter()
            .find(|page| {
                tab.as_ref().is_some_and(|tab| &page.tab_id == tab)
                    || target.is_some_and(|target| page.target_id.as_str() == target)
            })
            .ok_or_else(|| unsupported(method))?;
        if tab.as_ref().is_some_and(|tab| &selected.tab_id != tab)
            || target.is_some_and(|target| selected.target_id.as_str() != target)
        {
            return Err(unsupported(method));
        }
        self.page(selected.page_id).await
    }
    async fn validate_tabs(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Vec<PageInfo>, CoreError> {
        let tabs = params["tabIds"]
            .as_array()
            .ok_or_else(|| unsupported(method))?;
        if tabs.is_empty() {
            return Err(unsupported(method));
        }
        let all = self
            .session
            .pages
            .list_with_epoch(self.epoch_guard()?)
            .await?;
        let mut admitted = Vec::new();
        for id in tabs {
            let id = id.as_i64().ok_or_else(|| unsupported(method))?;
            let page = all
                .iter()
                .find(|page| page.tab_id.0 == id)
                .ok_or_else(|| unsupported(method))?;
            admitted.push(self.page(page.page_id.clone()).await?);
        }
        Ok(admitted)
    }
    pub(super) async fn validate_group(
        &self,
        id: &str,
        expected_window: Option<WindowId>,
    ) -> Result<Value, CoreError> {
        let value = self
            .send_at_capture("Browser.getTabGroups", json!({}), None)
            .await?;
        let group = value["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|group| group["groupId"] == id)
            .ok_or_else(|| TargetError::GroupOutOfScope(id.into()))?
            .clone();
        let window = group["windowId"]
            .as_i64()
            .map(WindowId)
            .ok_or_else(|| TargetError::GroupOutOfScope(id.into()))?;
        if expected_window.is_some_and(|expected| expected != window) {
            return Err(TargetError::GroupOutOfScope(id.into()).into());
        }
        if self.validate_window(window).await.is_err() {
            return Err(TargetError::GroupOutOfScope(id.into()).into());
        }
        let tabs = self
            .validate_tabs("Browser.getTabGroups", &json!({"tabIds":group["tabIds"]}))
            .await?;
        if tabs.is_empty()
            || tabs
                .iter()
                .any(|tab| tab.window_id != Some(window) || tab.group_id.as_deref() != Some(id))
        {
            return Err(TargetError::GroupOutOfScope(id.into()).into());
        }
        Ok(group)
    }
    fn inject_context(&self, method: &str, params: &mut Value) -> Result<(), CoreError> {
        let scope = self.scope()?.ok_or_else(|| unsupported(method))?;
        let context = scope.context()?;
        if params
            .get("browserContextId")
            .is_some_and(|id| id.as_str() != Some(context.as_str()))
        {
            return Err(unsupported(method));
        }
        params["browserContextId"] = json!(context.as_str());
        Ok(())
    }
    async fn placement(&self, method: &str, params: &mut Value) -> Result<WindowId, CoreError> {
        let scope = self.scope()?.ok_or_else(|| unsupported(method))?;
        let window = match params.get("windowId") {
            Some(id) => WindowId(id.as_i64().ok_or_else(|| unsupported(method))?),
            None => scope.placement_window.ok_or_else(|| {
                TargetError::NoProfileWindow(scope.profile.browser_profile_id.clone())
            })?,
        };
        self.validate_window(window).await?;
        params["windowId"] = json!(window.0);
        Ok(window)
    }
    pub(super) async fn scoped_root_cdp(
        &self,
        method: &str,
        mut params: Value,
    ) -> Result<Value, CoreError> {
        self.validate_live().await?;
        let allowed = root_parameters(method).ok_or_else(|| unsupported(method))?;
        validate_parameters(method, &params, allowed)?;
        let mut admission = DispatchAdmission::default();
        match method {
            "Browser.getVersion" | "Browser.getWindows" | "Target.getTargets" => {}
            "Browser.getTabs" | "Browser.getTabGroups" => {
                if let Some(id) = params.get("windowId") {
                    let window = WindowId(id.as_i64().ok_or_else(|| unsupported(method))?);
                    self.validate_window(window).await?;
                    admission.windows.push(window);
                }
            }
            "Browser.getTabInfo"
            | "Browser.closeTab"
            | "Browser.activateTab"
            | "Browser.showTab" => {
                let page = self.native_tab(method, &params).await?;
                // Address the admitted target incarnation, rather than a tab integer that may
                // have been rebound during an awaited inventory read. Native target resolution
                // also prevents fallback to whichever window happens to be focused.
                use_target_selector(method, &mut params, &page)?;
                if method == "Browser.showTab" {
                    let window = match params.get("windowId") {
                        Some(id) => WindowId(id.as_i64().ok_or_else(|| unsupported(method))?),
                        None => page.window_id.ok_or_else(|| unsupported(method))?,
                    };
                    self.validate_window(window).await?;
                    admission.windows.push(window);
                    params["windowId"] = json!(window.0);
                }
                admission.tabs.push(page);
            }
            "Browser.getActiveTab" => {
                admission
                    .windows
                    .push(self.placement(method, &mut params).await?);
            }
            "Browser.moveTab" => {
                let page = self.native_tab(method, &params).await?;
                if params.get("windowId").is_none() {
                    params["windowId"] =
                        json!(page.window_id.ok_or_else(|| unsupported(method))?.0);
                }
                let window = WindowId(
                    params["windowId"]
                        .as_i64()
                        .ok_or_else(|| unsupported(method))?,
                );
                self.validate_window(window).await?;
                admission.windows.push(window);
                use_target_selector(method, &mut params, &page)?;
                admission.tabs.push(page);
            }
            "Browser.createTab" => {
                self.inject_context(method, &mut params)?;
                let scope = self.scope()?.ok_or_else(|| unsupported(method))?;
                if params.get("windowId").is_none()
                    && scope.placement_window.is_none()
                    && scope.source == super::TargetSource::Default
                {
                    // Supply only the placement; every other argument the caller passed stays.
                    params["windowId"] = json!(Box::pin(self.create_default_window()).await?.0);
                }
                admission
                    .windows
                    .push(self.placement(method, &mut params).await?);
                params["background"] = json!(true);
            }
            "Browser.createWindow" => {
                let scope = self.scope()?.ok_or_else(|| unsupported(method))?;
                if scope.pin().is_some() {
                    return Err(TargetError::PinnedRestricts.into());
                }
                if params
                    .get("windowType")
                    .is_some_and(|value| value != "normal")
                    || params.get("hidden").is_some_and(|value| value != false)
                {
                    return Err(unsupported(method));
                }
                self.inject_context(method, &mut params)?;
                params["windowType"] = json!("normal");
            }
            "Browser.closeWindow" | "Browser.activateWindow" | "Browser.setWindowVisibility" => {
                let window = WindowId(
                    params["windowId"]
                        .as_i64()
                        .ok_or_else(|| unsupported(method))?,
                );
                self.validate_window(window).await?;
                admission.windows.push(window);
            }
            "Browser.createTabGroup" | "Browser.addTabsToGroup" | "Browser.removeTabsFromGroup" => {
                let tabs = self.validate_tabs(method, &params).await?;
                let window = tabs.first().and_then(|tab| tab.window_id);
                if method != "Browser.removeTabsFromGroup"
                    && tabs.iter().any(|tab| tab.window_id != window)
                {
                    return Err(unsupported(method));
                }
                if method != "Browser.removeTabsFromGroup" {
                    admission.group_window = window;
                }
                if method == "Browser.addTabsToGroup" {
                    let id = params["groupId"]
                        .as_str()
                        .ok_or_else(|| unsupported(method))?;
                    let group = self.validate_group(id, window).await?;
                    admission.tabs.extend(
                        self.validate_tabs(method, &json!({"tabIds":group["tabIds"]}))
                            .await?,
                    );
                    admission.groups.push(group);
                }
                if method == "Browser.removeTabsFromGroup" {
                    for tab in &tabs {
                        if let Some(id) = &tab.group_id {
                            self.validate_group(id, tab.window_id).await?;
                        }
                    }
                }
                admission.tabs.extend(tabs);
            }
            "Browser.updateTabGroup" | "Browser.closeTabGroup" | "Browser.moveTabGroup" => {
                let id = params["groupId"]
                    .as_str()
                    .ok_or_else(|| unsupported(method))?;
                let group = self.validate_group(id, None).await?;
                admission.tabs.extend(
                    self.validate_tabs(method, &json!({"tabIds":group["tabIds"]}))
                        .await?,
                );
                if method == "Browser.moveTabGroup" {
                    let destination = params
                        .get("windowId")
                        .unwrap_or(&group["windowId"])
                        .as_i64()
                        .ok_or_else(|| unsupported(method))?;
                    self.validate_window(WindowId(destination)).await?;
                    admission.windows.push(WindowId(destination));
                    params["windowId"] = json!(destination);
                }
                admission.groups.push(group);
            }
            "History.search"
            | "History.getRecent"
            | "History.deleteUrl"
            | "History.deleteRange"
            | "Storage.getCookies"
            | "Storage.setCookies"
            | "Storage.clearCookies"
            | "Browser.setPermission"
            | "Browser.grantPermissions"
            | "Browser.resetPermissions"
            | "Browser.setDownloadBehavior" => {
                self.inject_context(method, &mut params)?;
            }
            "Target.createTarget" => return self.create_target(params).await,
            _ => return Err(unsupported(method)),
        }
        // Revalidate after method-specific awaited lookups and before mutation. Native creation
        // receives genuine context + window IDs; native move guards run before any detachment.
        self.admit_dispatch(&admission).await?;
        let result = self.send_at_capture(method, params, None).await?;
        // Closing one's own strict pin is allowed, but all later calls must fail closed.
        if method != "Browser.closeWindow"
            && method != "Browser.closeTab"
            && method != "Browser.closeTabGroup"
        {
            self.validate_live().await?;
        }
        self.project_reply(method, result).await
    }
    async fn project_reply(&self, method: &str, mut result: Value) -> Result<Value, CoreError> {
        match method {
            "Browser.getWindows" => {
                let live = self
                    .validate_live()
                    .await?
                    .ok_or_else(|| unsupported(method))?;
                let admitted = live
                    .windows
                    .iter()
                    .filter(|window| live.scope.window_matches(window))
                    .map(|window| window.info.window_id)
                    .collect::<BTreeSet<_>>();
                if let Some(windows) = result["windows"].as_array_mut() {
                    windows.retain(|window| {
                        window["windowId"]
                            .as_i64()
                            .is_some_and(|id| admitted.contains(&id))
                            && window["windowType"] == "normal"
                            && window["browserProfileId"]
                                == live.scope.profile.browser_profile_id.as_str()
                            && window["liveBrowserContextId"].as_str()
                                == live.scope.context_id.as_ref().map(BrowserContextId::as_str)
                    });
                }
            }
            "Browser.getTabs" | "Target.getTargets" | "Browser.getTabGroups" => {
                let listing = self.list_pages().await?;
                let live = self
                    .validate_live()
                    .await?
                    .ok_or_else(|| unsupported(method))?;
                match method {
                    "Browser.getTabs" => {
                        if let Some(tabs) = result["tabs"].as_array_mut() {
                            tabs.retain(|tab| {
                                listing.pages.iter().any(|page| {
                                    tab["tabId"] == page.tab_id.0
                                        && tab["targetId"] == page.target_id.as_str()
                                        && tab["browserProfileId"]
                                            == live.scope.profile.browser_profile_id.as_str()
                                        && tab["liveBrowserContextId"].as_str()
                                            == live
                                                .scope
                                                .context_id
                                                .as_ref()
                                                .map(BrowserContextId::as_str)
                                        && tab["windowId"].as_i64()
                                            == page.window_id.map(|window| window.0)
                                })
                            });
                        }
                    }
                    "Target.getTargets" => {
                        if let Some(targets) = result["targetInfos"].as_array_mut() {
                            targets.retain(|target| {
                                target["type"] == "page"
                                    && target["browserContextId"].as_str()
                                        == live
                                            .scope
                                            .context_id
                                            .as_ref()
                                            .map(BrowserContextId::as_str)
                                    && listing
                                        .pages
                                        .iter()
                                        .any(|page| target["targetId"] == page.target_id.as_str())
                            });
                        }
                    }
                    _ => {
                        if let Some(groups) = result["groups"].as_array_mut() {
                            groups.retain(|group| {
                                group["windowId"].as_i64().is_some_and(|id| {
                                    live.windows.iter().any(|window| {
                                        window.info.window_id == id
                                            && live.scope.window_matches(window)
                                    })
                                }) && group["tabIds"].as_array().is_some_and(|tabs| {
                                    !tabs.is_empty()
                                        && tabs.iter().all(|tab| {
                                            listing.pages.iter().any(|page| {
                                                *tab == page.tab_id.0
                                                    && page.window_id.map(|window| window.0)
                                                        == group["windowId"].as_i64()
                                            })
                                        })
                                })
                            });
                        }
                    }
                }
            }
            "Browser.getTabInfo"
            | "Browser.getActiveTab"
            | "Browser.createTab"
            | "Browser.moveTab" => {
                if result["tab"].is_null() {
                    return Ok(result);
                }
                let listing = self.list_pages().await?;
                let scope = self.scope()?.ok_or_else(|| unsupported(method))?;
                if !listing.pages.iter().any(|page| {
                    result["tab"]["targetId"] == page.target_id.as_str()
                        && result["tab"]["tabId"] == page.tab_id.0
                        && result["tab"]["browserProfileId"]
                            == scope.profile.browser_profile_id.as_str()
                        && result["tab"]["liveBrowserContextId"].as_str()
                            == scope.context_id.as_ref().map(BrowserContextId::as_str)
                        && result["tab"]["windowId"].as_i64()
                            == page.window_id.map(|window| window.0)
                }) {
                    return Err(unsupported(method));
                }
            }
            "Browser.createWindow" => {
                let window = result["window"]["windowId"]
                    .as_i64()
                    .map(WindowId)
                    .ok_or_else(|| unsupported(method))?;
                self.validate_window(window).await?;
            }
            "Browser.createTabGroup"
            | "Browser.addTabsToGroup"
            | "Browser.updateTabGroup"
            | "Browser.moveTabGroup" => {
                let id = result["group"]["groupId"]
                    .as_str()
                    .ok_or_else(|| unsupported(method))?;
                self.validate_group(id, None).await?;
            }
            _ => {}
        }
        Ok(result)
    }
    async fn create_target(&self, params: Value) -> Result<Value, CoreError> {
        const METHOD: &str = "Target.createTarget";
        let scope = self.scope()?.ok_or_else(|| unsupported(METHOD))?;
        if params.get("hidden").is_some_and(|value| value != false)
            || params.get("forTab").is_some_and(|value| value != false)
            || params
                .get("enableBeginFrameControl")
                .is_some_and(|value| value != false)
        {
            return Err(unsupported(METHOD));
        }
        if params.get("browserContextId").is_some_and(|id| {
            id.as_str() != scope.context_id.as_ref().map(BrowserContextId::as_str)
        }) {
            return Err(unsupported(METHOD));
        }
        let url = params["url"].as_str().ok_or_else(|| unsupported(METHOD))?;
        if params
            .get("newWindow")
            .is_some_and(|value| value != true && value != false)
        {
            return Err(unsupported(METHOD));
        }
        let created = if params["newWindow"] == true {
            if scope.pin().is_some() {
                return Err(TargetError::PinnedRestricts.into());
            }
            let mut options = json!({"url":url});
            let mut bounds = serde_json::Map::new();
            for key in ["left", "top", "width", "height", "windowState"] {
                if let Some(value) = params.get(key) {
                    bounds.insert(key.into(), value.clone());
                }
            }
            if !bounds.is_empty() {
                options["bounds"] = Value::Object(bounds);
            }
            let window = Box::pin(self.scoped_root_cdp("Browser.createWindow", options)).await?;
            Box::pin(self.scoped_root_cdp(
                "Browser.getActiveTab",
                json!({"windowId":window["window"]["windowId"]}),
            ))
            .await?
        } else {
            if ["left", "top", "width", "height", "windowState"]
                .iter()
                .any(|key| params.get(key).is_some())
            {
                return Err(unsupported(METHOD));
            }
            Box::pin(
                self.scoped_root_cdp("Browser.createTab", json!({"url":url,"background":true})),
            )
            .await?
        };
        let id = created["tab"]["targetId"]
            .as_str()
            .ok_or_else(|| unsupported(METHOD))?;
        Ok(json!({"targetId":id}))
    }
}
