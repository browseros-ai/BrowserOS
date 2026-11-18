//! Obstruction refs and parent-frame admission share the observer's document scope.

use super::{Observer, SnapshotBudget, known_main_frame_changed, should_reset_refs};
use crate::{CoreError, FrameId, ProtocolSession, Ref};
use serde::Deserialize;
use serde_json::json;

impl Observer {
    /// Give an obstruction a real node ref even when it has no interactive AX
    /// role. Native hit testing supplies its owning frame; the document scope is
    /// checked again before committing so navigation cannot mint a misleading ref.
    pub(crate) async fn reference_at_point(
        &self,
        session: &ProtocolSession,
        point: crate::input::Point,
    ) -> Result<Option<Ref>, CoreError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Hit {
            backend_node_id: i64,
            frame_id: FrameId,
        }
        let root = self.page_session().await?.session;
        let budget = SnapshotBudget::new();
        let before = self.read_main_frame_state(&root, &budget).await;
        let hit: Hit = session
            .send(
                "DOM.getNodeForLocation",
                json!({
                    "x": point.x.round() as i64, "y": point.y.round() as i64,
                    "includeUserAgentShadowDOM": true,
                }),
            )
            .await?;
        let frame_id = if before.frame_documents.get(&Some(hit.frame_id.clone()))
            == before.frame_documents.get(&None)
        {
            None
        } else {
            Some(hit.frame_id)
        };
        let mut document_id = before.frame_documents.get(&frame_id).cloned();
        if document_id.is_none() {
            // Remote frames may be absent from the root's tree. Their own CDP
            // session can still establish the document identity for this ref.
            let child = self.read_main_frame_state(session, &budget).await;
            document_id = child.frame_documents.get(&frame_id).cloned();
        }
        let Some(document_id) = document_id else {
            return Ok(None);
        };
        let after = self.read_main_frame_state(&root, &budget).await;
        if known_main_frame_changed(&before, &after) {
            return Err(CoreError::DocumentChanged);
        }
        let mut current_document = after.frame_documents.get(&frame_id).cloned();
        if current_document.is_none() {
            let child = self.read_main_frame_state(session, &budget).await;
            current_document = child.frame_documents.get(&frame_id).cloned();
        }
        if current_document.as_ref() != Some(&document_id) {
            return Err(CoreError::DocumentChanged);
        }
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        let mut state = self.state.lock().await;
        if should_reset_refs(state.ref_scope.as_ref(), &after) {
            return Err(CoreError::DocumentChanged);
        }
        Ok(Some(state.refs.mint_exact(
            crate::snapshot::refs::MintRef {
                backend_node_id: hit.backend_node_id,
                role: "generic",
                name: "",
                document_id: Some(&document_id),
                frame_id: frame_id.as_ref(),
            },
        )))
    }

    /// Parent visibility is separate from visibility inside an OOPIF. Follow
    /// the snapshot's frame ownership chain through the same scoped admission,
    /// never borrowing an unbound session to inspect a parent viewport.
    pub(crate) async fn click_parent(
        &self,
        session: &ProtocolSession,
    ) -> Result<Option<(ProtocolSession, i64)>, CoreError> {
        let root = self.page_session().await?.session;
        if root.same_session(session) {
            return Ok(None);
        }
        let child = match session.session_id() {
            Some(id) => self.frames.oopif_for_session(id).await,
            None => None,
        }
        .ok_or_else(|| {
            CoreError::Message("Click frame has no known owner; take a new snapshot.".into())
        })?;
        let link = self.frames.frame_link(&child).await.ok_or_else(|| {
            CoreError::Message("Click frame is detached; take a new snapshot.".into())
        })?;
        let parent = if root.session_id() == Some(&link.parent) {
            root
        } else {
            let frame = self
                .frames
                .oopif_for_session(&link.parent)
                .await
                .ok_or_else(|| {
                    CoreError::Message(
                        "Click frame parent is detached; take a new snapshot.".into(),
                    )
                })?;
            let mut target = self
                .frames
                .resolve_frame_target_with_epoch(
                    self.page_id.clone(),
                    Some(frame.clone()),
                    None,
                    false,
                    self.access
                        .as_ref()
                        .map(|access| access.epoch_guard())
                        .transpose()?
                        .flatten(),
                )
                .await?;
            if let Some(access) = &self.access {
                target.session = access.frame_session(target.session, Some(frame)).await?;
            }
            target.session
        };
        let owner = parent
            .send_value("DOM.getFrameOwner", json!({"frameId": child.as_str()}))
            .await?;
        let backend = owner["backendNodeId"]
            .as_i64()
            .ok_or_else(|| CoreError::Message("Click frame owner has no backend node.".into()))?;
        Ok(Some((parent, backend)))
    }
}
