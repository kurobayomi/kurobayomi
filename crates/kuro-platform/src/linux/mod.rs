//! Linux-specific integration: MPRIS, the screen-saver inhibit, disk space, the launcher badge, the hidden web pages, the GL entry points and keeping the window above others.

pub mod appearance;
pub mod device;
pub mod gl;
pub mod hidden_pages;
pub mod idle;
pub mod launcher;
pub mod media;
pub mod power_network;
pub mod storage;
pub mod window_level;

pub use idle::ScreenSaver;
pub use media::Mpris;
