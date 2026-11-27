#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod desktop;
mod subscriber;
mod view;
#[cfg(windows)]
fn main() {
    desktop::run();
}
#[cfg(not(windows))]
fn main() {
    eprintln!("The native dashboard requires Windows.");
}
