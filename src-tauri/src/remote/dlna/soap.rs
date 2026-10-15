//! UPnP control calls: a SOAP envelope posted to a service's control URL.

use std::time::Duration;

use super::Service;
use crate::remote::xml::escape;

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// No answer (gone, or not on the network any more).
    Unreachable,
    /// The device answered with a UPnP error (`errorCode`), e.g. 714 for a
    /// format it won't play.
    Fault(Option<u32>),
}

#[derive(Clone)]
pub struct Soap {
    http: reqwest::Client,
    timeout: Duration,
}

impl Soap {
    pub fn new(http: reqwest::Client, timeout: Duration) -> Self {
        Self { http, timeout }
    }

    /// Call `action` with `args` (in order); the answer's body.
    pub async fn call(&self, svc: &Service, action: &str, args: &[(&str, String)]) -> Result<String, CallError> {
        let ty = &svc.service_type;
        let args: String = args.iter().map(|(k, v)| format!("<{k}>{}</{k}>", escape(v))).collect();
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
<s:Body><u:{action} xmlns:u=\"{ty}\">{args}</u:{action}></s:Body></s:Envelope>"
        );
        let res = self
            .http
            .post(svc.control.clone())
            .timeout(self.timeout)
            .header("content-type", "text/xml; charset=\"utf-8\"")
            .header("soapaction", format!("\"{ty}#{action}\""))
            .body(body)
            .send()
            .await
            .map_err(|_| CallError::Unreachable)?;
        let ok = res.status().is_success();
        let text = res.text().await.map_err(|_| CallError::Unreachable)?;
        if ok {
            Ok(text)
        } else {
            Err(CallError::Fault(super::text(&text, "errorCode").and_then(|c| c.parse().ok())))
        }
    }
}
