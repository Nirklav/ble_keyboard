use esp32_nimble::{BLEError, BLEServer};

#[derive(Clone, Copy)]
pub struct ConnParams {
    pub min: u16,
    pub max: u16,
    pub latency: u16,
    pub timeout: u16
}

impl ConnParams {
    pub const IDLE: ConnParams = ConnParams::new(12, 24, 10, 70);
    pub const ACTIVE: ConnParams = ConnParams::new(6, 12, 0, 60);

    pub const fn new(min: u16, max: u16, latency: u16, timeout: u16) -> Self {
        Self {
            min,
            max,
            latency,
            timeout
        }
    }

    pub fn set(&self, server: &mut BLEServer, conn_handle: u16) -> Result<(), BLEError> {
        server.update_conn_params(conn_handle, self.min, self.max, self.latency, self.timeout)?;
        Ok(())
    }
}