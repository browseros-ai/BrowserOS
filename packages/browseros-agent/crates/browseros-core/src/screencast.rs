use crate::{CoreError, ProtocolSession, screenshot::ScreenshotFormat};
use browseros_cdp::page;
use serde_json::Value;

pub use browseros_cdp::page::{ScreencastFrameEvent, ScreencastFrameMetadata};

/// The `Page.screencastFrame` event method name, for routing off a raw event stream.
pub const SCREENCAST_FRAME_METHOD: &str = "Page.screencastFrame";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreencastOptions {
    pub format: ScreenshotFormat,
    pub quality: i64,
    pub max_width: i64,
    pub max_height: i64,
    pub every_nth_frame: i64,
}

impl Default for ScreencastOptions {
    fn default() -> Self {
        Self {
            format: ScreenshotFormat::Jpeg,
            quality: 60,
            max_width: 1280,
            max_height: 800,
            every_nth_frame: 1,
        }
    }
}

impl ScreencastOptions {
    #[must_use]
    pub fn to_params(&self) -> page::StartScreencastParams {
        page::StartScreencastParams {
            format: Some(self.format.as_str().to_string()),
            quality: Some(self.quality),
            max_width: Some(self.max_width),
            max_height: Some(self.max_height),
            every_nth_frame: Some(self.every_nth_frame),
        }
    }
}

pub async fn start_screencast(
    session: &ProtocolSession,
    options: &ScreencastOptions,
) -> Result<(), CoreError> {
    let _: page::StartScreencastResult = session
        .send("Page.startScreencast", options.to_params())
        .await?;
    Ok(())
}

pub async fn stop_screencast(session: &ProtocolSession) -> Result<(), CoreError> {
    let _: page::StopScreencastResult = session
        .send("Page.stopScreencast", serde_json::json!({}))
        .await?;
    Ok(())
}

/// Ack a received frame with the integer cookie from the event params —
/// Chrome stops sending frames until the previous one is acked.
pub async fn ack_frame(session: &ProtocolSession, frame_session_id: i64) -> Result<(), CoreError> {
    let _: page::ScreencastFrameAckResult = session
        .send(
            "Page.screencastFrameAck",
            page::ScreencastFrameAckParams {
                session_id: frame_session_id,
            },
        )
        .await?;
    Ok(())
}

/// Parse `Page.screencastFrame` event params, consuming them so the frame
/// data string is moved, not copied. The `session_id` inside is the ack
/// cookie (integer), unrelated to the envelope target session id (string).
#[must_use]
pub fn parse_frame_event(params: Value) -> Option<ScreencastFrameEvent> {
    serde_json::from_value(params).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_options_match_claw_screencast_params() {
        let options = ScreencastOptions::default();
        assert_eq!(options.format, ScreenshotFormat::Jpeg);
        assert_eq!(options.quality, 60);
        assert_eq!(options.max_width, 1280);
        assert_eq!(options.max_height, 800);
        assert_eq!(options.every_nth_frame, 1);
    }

    #[test]
    fn start_params_serialize_to_cdp_wire_shape() -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(ScreencastOptions::default().to_params())?;
        assert_eq!(
            value,
            json!({
                "format": "jpeg",
                "quality": 60,
                "maxWidth": 1280,
                "maxHeight": 800,
                "everyNthFrame": 1
            })
        );
        Ok(())
    }

    #[test]
    fn frame_event_deserializes_with_optional_timestamp() {
        let params = json!({
            "data": "aGVsbG8=",
            "metadata": {
                "offsetTop": 0.0,
                "pageScaleFactor": 1.0,
                "deviceWidth": 1280.0,
                "deviceHeight": 800.0,
                "scrollOffsetX": 0.0,
                "scrollOffsetY": 12.5
            },
            "sessionId": 7
        });
        let event = parse_frame_event(params.clone());
        let Some(event) = event else {
            panic!("expected frame event to parse: {params}");
        };
        assert_eq!(event.data, "aGVsbG8=");
        assert_eq!(event.session_id, 7);
        assert_eq!(event.metadata.timestamp, None);
        assert!((event.metadata.scroll_offset_y - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn frame_event_parse_rejects_garbage() {
        assert!(parse_frame_event(json!({ "data": 42 })).is_none());
        assert!(parse_frame_event(json!("not an object")).is_none());
        assert!(parse_frame_event(json!({})).is_none());
    }
}
