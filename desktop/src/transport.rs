//! Native connection to the keyboard config reports ([`protocol::hid`]) through hidapi.
//!
//! The OS exposes the config collection as a HID device whenever the keyboard is connected (over BLE
//! too: HID over GATT), so finding the keyboard is a lookup of its usage page. On Linux the user needs
//! access to the hidraw device: a udev rule, see [`UDEV_RULE`].
//!
//! hidapi is blocking: a request blocks the calling task up to the timeout, so the apps run the
//! device on a thread of its own (the GUI worker) or have nothing else to do (the CLI).

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hidapi::{HidApi, HidDevice};
use protocol::hid::{self, Request, Response};

use crate::link::{Link, Options, encode, name};

/// Gives the user at the computer access to the keyboard's hidraw device: Bluetooth bus (0005),
/// [`hid::VENDOR_ID`], [`hid::PRODUCT_ID`].
pub const UDEV_RULE: &str = r#"KERNEL=="hidraw*", KERNELS=="0005:1209:0001.*", TAG+="uaccess""#;

/// Where [`UDEV_RULE`] goes.
pub const UDEV_RULE_FILE: &str = "/etc/udev/rules.d/70-nirklav-keyboard.rules";

/// Read buffer: the report ID and the report, with a spare byte to notice longer reports.
const READ_LEN: usize = 1 + hid::REPORT_LEN + 1;

pub struct Device {
    device: HidDevice,
    timeout: Duration,
}

impl Device {
    /// Opens the config collection of the connected keyboard.
    pub async fn connect(options: &Options) -> Result<Device> {
        let api = HidApi::new().context("can't list the HID devices")?;
        let info = api
            .device_list()
            .find(|info| info.usage_page() == hid::USAGE_PAGE && info.usage() == hid::USAGE)
            .ok_or_else(|| {
                anyhow!("keyboard not found: check that it is paired and connected, and its firmware is up to date")
            })?;
        let device = info.open_device(&api).map_err(|error| {
            let error = anyhow!("can't open the keyboard: {error}");
            if cfg!(target_os = "linux") {
                error.context(format!(
                    "no access to the keyboard: add the udev rule `kb udev-rule` prints to {UDEV_RULE_FILE} \
                     and reconnect the keyboard"
                ))
            } else {
                error
            }
        })?;
        Ok(Device { device, timeout: options.timeout })
    }
}

impl Link for Device {
    async fn request_once(&mut self, request: &Request) -> Result<Response> {
        let report = encode(request)?;
        let mut output = [0; 1 + hid::REPORT_LEN];
        output[0] = hid::REPORT_ID;
        output[1..].copy_from_slice(&report);

        // Late responses of timed out requests.
        let mut input = [0; READ_LEN];
        while self.device.read_timeout(&mut input, 0).context("can't read from the keyboard")? > 0 {}

        self.device.write(&output).with_context(|| format!("can't send {}", name(request)))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("no response to {} within {:?}", name(request), self.timeout);
            }
            let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let len = self.device.read_timeout(&mut input, millis).context("can't read from the keyboard")?;
            // On Linux the hidraw device has the keyboard and mouse reports too.
            if len == 0 || input[0] != hid::REPORT_ID {
                continue;
            }
            let message = Some(&input[1..len])
                .filter(|report| report.len() <= hid::REPORT_LEN)
                .and_then(hid::decode)
                .ok_or_else(|| anyhow!("bad response report {:02x?}", &input[..len]))?;
            return Response::from_bytes(message).context("can't decode response");
        }
    }

    async fn close(mut self) {
        let _ = self.request_ok(&Request::End).await;
    }
}
