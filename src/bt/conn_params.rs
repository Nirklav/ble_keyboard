use esp32_nimble::{BLEError, BLEServer};

#[derive(Clone, Copy)]
pub struct ConnnParams {
    pub min: u16, 
    pub max: u16, 
    pub latency: u16
}

impl ConnnParams {
    pub const IDLE: ConnnParams = ConnnParams::new(12, 24, 10);
    pub const ACTIVE: ConnnParams = ConnnParams::new(6, 12, 0);

    pub const fn new(min: u16, max: u16, latency: u16) -> Self {
        Self {
            min,
            max,
            latency
        }
    }

    pub fn set(&self, server: &mut BLEServer, conn_handle: u16) -> Result<(), BLEError> {
        server.update_conn_params(conn_handle, self.min, self.max, self.latency, 60)?;
        Ok(())
    }
}