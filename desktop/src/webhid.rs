//! WebHID connection to the keyboard config reports ([`protocol::hid`]), for the browser build.
//!
//! The OS exposes the config collection as its own HID device whenever the keyboard is connected.
//! The browser asks the user to pick it once (`requestDevice` needs a user gesture: the Connect
//! click, the request runs within its activation), later the permission is remembered and
//! `getDevices` returns it without asking.
//!
//! Only Chromium browsers (Chrome, Edge, Opera) have WebHID.

use std::future::IntoFuture;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use futures::channel::mpsc;
use protocol::hid::{self, Request, Response};
use wasm_bindgen::prelude::*;
use web_sys::{HidDevice, HidDeviceFilter, HidDeviceRequestOptions, HidInputReportEvent};

use crate::link::{NotSent, Options, Transport, decode_response, encode, name};
use crate::time::timeout;

pub struct Device {
    device: HidDevice,
    /// Config input reports (without the report ID), from `on_report`.
    reports: mpsc::UnboundedReceiver<Vec<u8>>,
    /// `inputreport` listener, removed on drop.
    on_report: Closure<dyn FnMut(HidInputReportEvent)>,
    timeout: Duration,
}

impl Device {
    /// Opens the keyboard: the already permitted one, otherwise asks the user to pick it.
    pub async fn connect(options: &Options) -> Result<Device> {
        let window = web_sys::window().ok_or_else(|| anyhow!("no browser window"))?;
        let navigator = window.navigator();
        if !js_sys::Reflect::has(&navigator, &"hid".into()).unwrap_or(false) {
            bail!("this browser has no WebHID, use Chrome, Edge or Opera");
        }
        let hid_api = navigator.hid();

        let known = hid_api.get_devices().await.map_err(js_error).context("can't list the HID devices")?;
        let device = match known.iter().find(is_config_device) {
            Some(device) => device,
            None => {
                let filter = HidDeviceFilter::new();
                filter.set_usage_page(hid::USAGE_PAGE);
                filter.set_usage(hid::USAGE);
                let picked = hid_api
                    .request_device(&HidDeviceRequestOptions::new(&[filter]))
                    .await
                    .map_err(js_error)
                    .context("can't ask for the keyboard")?;
                picked.iter().find(is_config_device).ok_or_else(|| {
                    anyhow!("no keyboard selected: pick it in the browser dialog (it must be connected to the computer)")
                })?
            }
        };

        if !device.opened() {
            device.open().await.map_err(js_error).context("can't open the keyboard")?;
        }

        let (sender, reports) = mpsc::unbounded();
        let on_report = Closure::<dyn FnMut(HidInputReportEvent)>::new(move |event: HidInputReportEvent| {
            if event.report_id() != hid::REPORT_ID {
                return;
            }
            let data = event.data();
            let bytes = js_sys::Uint8Array::new_with_byte_offset_and_length(
                &data.buffer(),
                data.byte_offset() as u32,
                data.byte_length() as u32,
            );
            let _ = sender.unbounded_send(bytes.to_vec());
        });
        device
            .add_event_listener_with_callback("inputreport", on_report.as_ref().unchecked_ref())
            .map_err(js_error)
            .context("can't listen to the keyboard")?;
        Ok(Device { device, reports, on_report, timeout: options.timeout })
    }
}

impl Transport for Device {
    fn drain(&mut self) -> Result<()> {
        while self.reports.try_recv().is_ok() {}
        Ok(())
    }

    async fn send(&mut self, request: &Request) -> Result<()> {
        let mut report = encode(request)?;
        let sent = self.device.send_report_with_u8_slice(hid::REPORT_ID, &mut report).map_err(js_error).context(NotSent(name(request)))?;
        timeout(self.timeout, sent.into_future())
            .await
            .with_context(|| format!("{} send timed out", name(request)))?
            .map_err(js_error)
            .context(NotSent(name(request)))?;
        Ok(())
    }

    async fn receive(&mut self, name: &'static str) -> Result<Response> {
        let report = timeout(self.timeout, self.reports.next())
            .await
            .with_context(|| format!("no response to {name} within {:?}", self.timeout))?
            .ok_or_else(|| anyhow!("keyboard disconnected"))?;
        let message = hid::decode(&report).ok_or_else(|| anyhow!("bad response report {report:02x?}"))?;
        decode_response(message)
    }

    async fn release(self) {
        let _ = timeout(self.timeout, self.device.close().into_future()).await;
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.device.remove_event_listener_with_callback("inputreport", self.on_report.as_ref().unchecked_ref());
    }
}

/// The browser gives one device per top-level collection.
fn is_config_device(device: &HidDevice) -> bool {
    device.collections().iter().any(|collection| {
        collection.get_usage_page() == Some(hid::USAGE_PAGE) && collection.get_usage() == Some(hid::USAGE)
    })
}

/// Text of a JS exception: `name: message` of a `DOMException` / `Error`.
fn js_error(value: JsValue) -> anyhow::Error {
    if let Some(error) = value.dyn_ref::<web_sys::DomException>() {
        return anyhow!("{}: {}", error.name(), error.message());
    }
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return anyhow!("{}", String::from(error.to_string()));
    }
    anyhow!("{value:?}")
}
