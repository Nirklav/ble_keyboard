//! Wire protocols shared by the keyboard firmware, its extensions and the desktop app.
//!
//! Features:
//! - none: [`joystick`], [`keys`], [`crc`] and the [`ble`] constants, usable without an allocator;
//! - `alloc`: [`config`] model and the [`ble`] config protocol messages;
//! - `std`: `alloc` + `std::error::Error` impls.
#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod ble;
pub mod crc;
pub mod joystick;
pub mod keys;

#[cfg(feature = "alloc")]
pub mod config;
