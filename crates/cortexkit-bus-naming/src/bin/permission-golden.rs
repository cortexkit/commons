use cortexkit_bus_naming::{generate_permission_golden, PINNED_GOLDEN_FIXTURE};

fn main() {
    let document = generate_permission_golden(PINNED_GOLDEN_FIXTURE)
        .expect("the pinned permission fixture must remain valid");
    print!("{}", document.render());
}
