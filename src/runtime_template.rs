// The shipped runtime is the same source file that `src/runtime.rs` is
// unit-tested against — there is no separate copy to drift out of sync.
pub const RUNTIME_TEMPLATE: &str = include_str!("runtime.rs");
