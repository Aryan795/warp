#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_family = "wasm")]
pub mod wasm;
#[cfg(windows)]
pub mod windows;

pub fn init() {
    #[cfg(target_os = "linux")]
    linux::maybe_reexec_on_flatpak_host();
    #[cfg(target_family = "wasm")]
    wasm::init();
}
