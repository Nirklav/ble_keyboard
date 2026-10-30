use core::ffi::{c_int, c_void};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnParams {
    pub min: u16,
    pub max: u16,
    pub latency: u16,
    pub timeout: u16
}

impl ConnParams {
    pub const IDLE: ConnParams = ConnParams::new(12, 24, 10, 70);
    pub const ACTIVE: ConnParams = ConnParams::new(6, 12, 0, 60);
    /// While a firmware update is received: the shortest interval, a chunk takes about two of them.
    pub const TRANSFER: ConnParams = ConnParams::new(6, 6, 0, 60);

    pub const fn new(min: u16, max: u16, latency: u16, timeout: u16) -> Self {
        Self {
            min,
            max,
            latency,
            timeout
        }
    }

    /// Params for the main loop's state: idle, or receiving a firmware update while active.
    pub fn of(idle: bool, transfer: bool) -> ConnParams {
        match (idle, transfer) {
            (true, _) => ConnParams::IDLE,
            (false, true) => ConnParams::TRANSFER,
            (false, false) => ConnParams::ACTIVE,
        }
    }

    /// Interval of connection `conn_handle` in milliseconds, `None` if there's no such connection.
    pub fn interval_ms(conn_handle: u16) -> Option<f32> {
        let mut desc = esp_idf_sys::ble_gap_conn_desc::default();
        if unsafe { esp_idf_sys::ble_gap_conn_find(conn_handle, &mut desc) } != 0 {
            return None;
        }
        Some(desc.conn_itvl as f32 * 1.25)
    }

    /// Connection `conn_handle` runs with these params already. `None` if there's no such connection.
    pub fn in_use(&self, conn_handle: u16) -> Option<bool> {
        let mut desc = esp_idf_sys::ble_gap_conn_desc::default();
        if unsafe { esp_idf_sys::ble_gap_conn_find(conn_handle, &mut desc) } != 0 {
            return None;
        }
        Some(
            (self.min..=self.max).contains(&desc.conn_itvl)
                && desc.conn_latency == self.latency
                && desc.supervision_timeout == self.timeout,
        )
    }

    /// Asks the host for these params with an L2CAP request, not the LL procedure: Windows doesn't always
    /// answer the LL one, and its LL response timeout drops the connection (0x22, after 40 s). An L2CAP
    /// request it doesn't answer fails after 30 s, `done` is called then, or on the answer, or when the
    /// connection is gone. NimBLE runs one request at a time (`BLE_HS_ENOMEM` while one is pending),
    /// and its slot is still taken while `done` runs.
    pub fn request(
        &self,
        conn_handle: u16,
        done: unsafe extern "C" fn(u16, c_int, *mut c_void),
        arg: *mut c_void,
    ) -> c_int {
        let mut params = esp_idf_sys::ble_l2cap_sig_update_params {
            itvl_min: self.min,
            itvl_max: self.max,
            slave_latency: self.latency,
            timeout_multiplier: self.timeout,
        };
        unsafe { esp_idf_sys::ble_l2cap_sig_update(conn_handle, &mut params, Some(done), arg) }
    }
}
