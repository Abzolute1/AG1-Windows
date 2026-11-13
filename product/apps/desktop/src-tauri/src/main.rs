// AG1W has its own composition host. The original desktop host remains intact
// for Linux, where its TypeScript worker and Linux-only integrations exist.
#[cfg(not(windows))]
mod main_linux;
#[cfg(windows)]
mod main_windows;

#[cfg(windows)]
fn main() {
    main_windows::run();
}

#[cfg(not(windows))]
fn main() {
    main_linux::run();
}
