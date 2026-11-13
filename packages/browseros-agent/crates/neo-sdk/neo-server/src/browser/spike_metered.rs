//! Spike only (branch `feat/chromium-tree-spike`, never merged): the CDP transport, wrapped so
//! `neo_playwright::spike_cdp_meter` can count each call's JSON bytes while a desktop snapshot
//! runs. Every method forwards unchanged; counting happens only while the meter is on, because
//! it re-encodes the params and the reply.

use std::io;
use std::sync::Arc;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use neo_playwright::spike_cdp_meter;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// [`CdpConnection`] over `inner`, counted.
pub(super) struct MeteredConnection {
    pub(super) inner: Arc<dyn CdpConnection>,
}

/// The JSON length of `value`, without building the string.
fn json_len(value: &Value) -> usize {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}

fn count_value(method: &str, params: &Value, reply: &Result<Value, CdpError>) {
    if spike_cdp_meter::enabled() {
        let received = reply.as_ref().map_or(0, json_len);
        spike_cdp_meter::record(method, json_len(params), received);
    }
}

fn count_raw(method: &str, params: &str, reply: &Result<String, CdpError>) {
    if spike_cdp_meter::enabled() {
        let received = reply.as_ref().map_or(0, String::len);
        spike_cdp_meter::record(method, params.len(), received);
    }
}

impl CdpConnection for MeteredConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            let counted = spike_cdp_meter::enabled().then(|| params.clone());
            let reply = self.inner.send(method, params, session).await;
            if let Some(params) = counted {
                count_value(method, &params, &reply);
            }
            reply
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let reply = self.inner.send_raw_json(method, params_json, session).await;
            count_raw(method, params_json, &reply);
            reply
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            let counted = spike_cdp_meter::enabled().then(|| params.clone());
            let reply = self
                .inner
                .send_at_epoch(expected_epoch, method, params, session)
                .await;
            if let Some(params) = counted {
                count_value(method, &params, &reply);
            }
            reply
        })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let reply = self
                .inner
                .send_raw_json_at_epoch(expected_epoch, method, params_json, session)
                .await;
            count_raw(method, params_json, &reply);
            reply
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.inner.events_targeted(method)
    }

    fn events_broadcast(&self) -> u64 {
        self.inner.events_broadcast()
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        self.inner.events_counted()
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        self.inner.events_targeted_counted(method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        self.inner.events_targeted_delivered(method)
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.inner.connection_epoch()
    }
}
