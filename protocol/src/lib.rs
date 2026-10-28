//! Wire protocols shared by the keyboard firmware, its extensions and the desktop app.
//!
//! Features:
//! - none: [`joystick`], [`keys`], [`crc`] and the [`hid`] constants and report framing, usable without
//!   an allocator;
//! - `alloc`: [`config`] model, the [`keymap`] key processing and the [`hid`] config protocol messages;
//! - `std`: `alloc` + `std::error::Error` impls.
#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod crc;
pub mod hid;
pub mod joystick;
pub mod keys;

#[cfg(feature = "alloc")]
pub mod config;
#[cfg(feature = "alloc")]
pub mod keymap;
#[cfg(feature = "alloc")]
pub mod motion;
