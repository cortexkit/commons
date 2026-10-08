#![forbid(unsafe_code)]

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    subc_os::privacy_identity::trampoline_main(&args);
    panic!("the fixture must only be invoked as a privacy trampoline");
}
