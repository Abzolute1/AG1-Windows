fn main() {
    #[cfg(target_os = "linux")]
    match ag1_platform::process::helper_main() {
        Ok(code) => std::process::exit(code),
        Err(_) => {
            eprintln!("AG1 owned process helper failed");
            std::process::exit(125);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("Owned process helper is unavailable on this platform");
        std::process::exit(125);
    }
}
