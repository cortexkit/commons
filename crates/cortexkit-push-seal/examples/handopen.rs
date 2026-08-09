fn main() {
    let a: Vec<String> = std::env::args().collect();
    let hx = |s: &str| {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
    };
    match cortexkit_push_seal::open(&hx(&a[1]), &hx(&a[2])) {
        Ok(p) => println!("OPENED: {}", String::from_utf8_lossy(&p)),
        Err(e) => println!("REFUSED: {:?} wire={}", e, e.wire_code()),
    }
}
