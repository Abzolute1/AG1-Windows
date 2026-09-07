fn main() {
    println!("P11 native runtime started");
    if std::env::args().any(|arg| arg == "--crash") {
        panic!("P11 deliberate native crash");
    }
    println!("P11 native runtime completed");
}
