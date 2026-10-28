use std::sync::{Arc, Mutex, MutexGuard};

use esp_idf_svc::hal::delay::TickType_t;
use esp_idf_svc::hal::i2c::{I2cDriver, I2cError};
use esp_idf_svc::sys::EspError;

/// I2C bus shared by the display and the extensions.
/// The display is flushed from its own thread, so the driver is behind a mutex.
#[derive(Clone)]
pub struct SharedI2c {
    driver: Arc<Mutex<I2cDriver<'static>>>,
}

impl SharedI2c {
    pub fn new(driver: I2cDriver<'static>) -> Self {
        Self {
            driver: Arc::new(Mutex::new(driver))
        }
    }

    pub fn write_read(&self, addr: u8, bytes: &[u8], buffer: &mut [u8], timeout: TickType_t) -> Result<(), EspError> {
        self.lock().write_read(addr, bytes, buffer, timeout)
    }

    fn lock(&self) -> MutexGuard<'_, I2cDriver<'static>> {
        // A panic in another user doesn't break the driver itself.
        self.driver.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl embedded_hal_0_2::blocking::i2c::Write for SharedI2c {
    type Error = I2cError;

    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        embedded_hal_0_2::blocking::i2c::Write::write(&mut *self.lock(), addr, bytes)
    }
}
