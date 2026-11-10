//! The `Request` and `Response` commands behind `page.goto`'s answer (`spec/serve.yml`), served
//! on the session that recorded the navigation. Each waits on the session's [`NetworkLog`] for
//! the value Playwright's server would await (`server/network.ts` at 26137face), then answers it;
//! only `Response.body` sends CDP.
//!
//! Playwright runs these with `kNoTimeout`, raced against the page's closure
//! (`raceWithPageClosure`): the SDK sends timeout 0, and the call's [`Progress`] races the page's
//! closed token and the run's end.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::network::GetResponseBodyResult;
use browseros_cdp::{CdpError, SessionId};
use neo_playwright_protocol::Binary;
use neo_playwright_protocol::serve::{
    RequestCall, RequestRawRequestHeadersParams, RequestRawRequestHeadersResult,
    RequestResponseParams, RequestResponseResult, ResponseBodyParams, ResponseBodyResult,
    ResponseCall, ResponseHttpVersionParams, ResponseHttpVersionResult,
    ResponseRawResponseHeadersParams, ResponseRawResponseHeadersResult,
    ResponseSecurityDetailsParams, ResponseSecurityDetailsResult, ResponseServerAddrParams,
    ResponseServerAddrResult, ResponseSizesParams, ResponseSizesResult,
};
use serde_json::Value;

use crate::browser_representations::network::{
    self, HopId, LogError, NetworkLog, Refetch, Settled,
};
use crate::browser_representations::page::PageSession;
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::playwright_injected_methods;
use crate::method_router::encode_result;
use crate::playwright_methods::tab::PageError;
use crate::progress::{CallError, PlaywrightError, Progress};

/// A call on one hop's `Request` or `Response`.
#[derive(Debug)]
pub(crate) enum NetworkCall {
    Request(HopId, RequestCall),
    Response(HopId, ResponseCall),
}

/// Playwright's wording when Chromium no longer holds a body (`internalBody`).
const BODY_GONE: &str = "Response body is not available for a response that was navigated away \
                         from. Read response.body() before triggering any navigation.";

pub(crate) async fn serve(
    page: &Arc<PageSession>,
    call: NetworkCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let log = &page.network;
    match call {
        NetworkCall::Request(hop, RequestCall::Response(_)) => {
            let response = match wait(progress, log, |log| log.response(&hop)).await? {
                Settled::Response(response) => Some(response),
                Settled::NoResponse => None,
            };
            encode_result::<RequestResponseParams>(&RequestResponseResult { response })
        }
        NetworkCall::Request(hop, RequestCall::RawRequestHeaders(_)) => {
            let headers = wait(progress, log, |log| log.raw_request_headers(&hop)).await?;
            encode_result::<RequestRawRequestHeadersParams>(&RequestRawRequestHeadersResult {
                headers,
            })
        }
        NetworkCall::Response(hop, ResponseCall::RawResponseHeaders(_)) => {
            let headers = wait(progress, log, |log| log.raw_response_headers(&hop)).await?;
            encode_result::<ResponseRawResponseHeadersParams>(&ResponseRawResponseHeadersResult {
                headers,
            })
        }
        NetworkCall::Response(hop, ResponseCall::SecurityDetails(_)) => {
            let value = Some(log.security_details(&hop).map_err(log_error)?);
            encode_result::<ResponseSecurityDetailsParams>(&ResponseSecurityDetailsResult { value })
        }
        NetworkCall::Response(hop, ResponseCall::ServerAddr(_)) => {
            let value = log.server_addr(&hop).map_err(log_error)?;
            encode_result::<ResponseServerAddrParams>(&ResponseServerAddrResult { value })
        }
        NetworkCall::Response(hop, ResponseCall::HttpVersion(_)) => {
            let value = log.http_version(&hop).map_err(log_error)?;
            encode_result::<ResponseHttpVersionParams>(&ResponseHttpVersionResult { value })
        }
        NetworkCall::Response(hop, ResponseCall::Sizes(_)) => {
            let sizes = wait(progress, log, |log| log.sizes(&hop)).await?;
            encode_result::<ResponseSizesParams>(&ResponseSizesResult { sizes })
        }
        NetworkCall::Response(hop, ResponseCall::Body(_)) => {
            let read = wait(progress, log, |log| log.body(&hop)).await?;
            let reply = progress
                .race(page.send_command_to(
                    read.session.as_ref(),
                    cdp::network::GetResponseBody {
                        request_id: read.request_id.clone(),
                    },
                ))
                .await?
                .map_err(body_error)?;
            let mut binary = body(reply)?;
            if binary.is_empty()
                && let Some(refetch) = read.refetch.as_ref().filter(|refetch| {
                    // `contentLength ? +contentLength : undefined`, then `!expectedLength`.
                    refetch.content_length.as_deref().is_some_and(|length| {
                        let length = network::js_string_to_number(length);
                        !length.is_nan() && length != 0.0
                    })
                })
            {
                binary = progress
                    .race(load_again(page, read.session.as_ref(), refetch))
                    .await??;
            }
            encode_result::<ResponseBodyParams>(&ResponseBodyResult { binary })
        }
    }
}

/// Waits until the log answers, raced against the page's closure and the run's end.
async fn wait<T>(
    progress: &Progress,
    log: &NetworkLog,
    ready: impl FnMut(&NetworkLog) -> Result<Option<T>, LogError>,
) -> Result<T, CallError> {
    progress
        .race(log.wait(ready))
        .await?
        .map_err(|error| log_error(error).into())
}

fn log_error(error: LogError) -> PlaywrightError {
    PlaywrightError::error(error.to_string())
}

/// The body as the wire's `binary`, as `createResponseBodyCallback` builds the `Buffer`: base64
/// bodies decoded, text bodies as their UTF-8 bytes. An empty body whose `content-length` says
/// otherwise is loaded again only for a static subresource or a prefetch ([`load_again`]); a
/// navigation's document is neither.
fn body(reply: GetResponseBodyResult) -> Result<Binary, PlaywrightError> {
    let GetResponseBodyResult {
        body,
        base64_encoded,
    } = reply;
    if base64_encoded {
        // Decoded here, so a body Chromium encodes differently still reaches the SDK as one form.
        let bytes = STANDARD
            .decode(body.as_bytes())
            .map_err(|error| PlaywrightError::error(format!("response body: {error}")))?;
        return Ok(Binary(bytes));
    }
    Ok(Binary(body.into_bytes()))
}

/// Playwright: `server/chromium/crNetworkManager.ts` `InterceptableRequest.createResponseBodyCallback`: `const resource = await session.send('Network.loadNetworkResource', { url: request.url(), frameId: request.serviceWorker() ? undefined : request.frame()!._id, options: { disableCache: false, includeCredentials: true } });`
///
/// The body Chromium no longer holds for a static subresource or a prefetch, loaded again
/// through the browser's cache and read from its stream, as bytes like `body`'s answer. A load
/// that opens no stream answers an empty body, as Playwright's loop then concatenates nothing.
async fn load_again(
    page: &PageSession,
    session: Option<&SessionId>,
    refetch: &Refetch,
) -> Result<Binary, PlaywrightError> {
    let loaded = page
        .send_command_to(
            session,
            cdp::network::LoadNetworkResource {
                frame_id: Some(refetch.frame.clone()),
                url: refetch.url.clone(),
                options: cdp::network::LoadNetworkResourceOptions {
                    disable_cache: false,
                    include_credentials: true,
                },
            },
        )
        .await
        .map_err(body_error)?;
    let Some(stream) = loaded.resource.stream else {
        return Ok(Binary(Vec::new()));
    };
    let mut bytes = Vec::new();
    loop {
        let chunk = page
            .send_command_to(
                session,
                cdp::io::Read {
                    handle: stream.clone(),
                    offset: None,
                    size: None,
                },
            )
            .await
            .map_err(body_error)?;
        let data = chunk["data"].as_str().unwrap_or_default();
        if chunk["base64Encoded"].as_bool() == Some(true) {
            bytes.extend(
                STANDARD
                    .decode(data.as_bytes())
                    .map_err(|error| PlaywrightError::error(format!("response body: {error}")))?,
            );
        } else {
            bytes.extend_from_slice(data.as_bytes());
        }
        if chunk["eof"].as_bool() != Some(false) {
            let _closed = page
                .send_command_to(session, cdp::io::Close { handle: stream })
                .await;
            break;
        }
    }
    Ok(Binary(bytes))
}

fn body_error(error: CdpError) -> PlaywrightError {
    match error {
        CdpError::Protocol { message, .. } => PlaywrightError::error(format!(
            "Protocol error ({}): {message}\n{BODY_GONE}",
            cdp::network::GetResponseBody::METHOD
        )),
        error if playwright_injected_methods::classify(&error) == PageError::Closed => {
            PlaywrightError::target_closed()
        }
        error => PlaywrightError::error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(body: &str, base64_encoded: bool) -> GetResponseBodyResult {
        GetResponseBodyResult {
            body: body.to_string(),
            base64_encoded,
        }
    }

    #[test]
    fn tests_that_text_bodies_cross_as_their_utf8_bytes() {
        let binary = body(reply("héllo", false)).expect("a body");
        assert_eq!(
            binary.as_bytes(),
            "héllo".as_bytes(),
            "Buffer.from(body, 'utf8')"
        );
    }

    #[test]
    fn tests_that_base64_bodies_keep_their_bytes() {
        let binary = body(reply("//5BAMM=", true)).expect("a body");
        assert_eq!(binary.as_bytes(), [0xff, 0xfe, 0x41, 0x00, 0xc3]);
    }

    #[test]
    fn tests_that_an_empty_document_body_stays_empty() {
        let binary = body(reply("", false)).expect("a body");
        assert!(binary.is_empty(), "{binary:?}");
    }

    #[test]
    fn tests_that_a_body_that_does_not_decode_is_an_error() {
        let error = body(reply("%%%", true)).expect_err("bad base64");
        assert!(error.message.contains("response body"), "{error:?}");
    }

    #[test]
    fn tests_that_a_body_chromium_no_longer_holds_says_why() {
        let error = body_error(CdpError::Protocol {
            code: -32000,
            message: "No resource with given identifier found".to_string(),
        });
        assert!(error.message.contains("No resource"), "{error:?}");
        assert!(error.message.contains("navigated away"), "{error:?}");
    }
}
