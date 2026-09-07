//! Matched fake workload and repeated native resume/compaction measurements.
// The shared contract fixture deliberately supplies more helpers than this probe uses.
#[allow(dead_code, unused_imports)]
mod fixture {
    include!("../tests/support/session_fixture.rs");
    include!("runtime_probe/measure.rs");
}
fn main() {
    fixture::measure();
}
