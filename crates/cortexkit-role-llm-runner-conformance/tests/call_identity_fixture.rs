//! The kit keeps its own copy of the `tool-provider/v1` pins of its fixture
//! definitions, so a runner's suite never depends on the tool-provider
//! crate. This test, the only place the kit uses that crate, catches the
//! copy drifting from what the tool-provider crate computes.

use cortexkit_role_llm_runner_conformance::{
    call_identity::{ToolDefinitionFixture, CHANGED_DEFINITION, DISPATCHED_DEFINITION},
    SCRIPTED_TOOL,
};
use cortexkit_role_tool_provider::{call::SchemaPin, catalog::schema_digest};

fn computed(fixture: &ToolDefinitionFixture) -> String {
    let digest = schema_digest(&fixture.input_schema()).expect("fixture schemas canonicalize");
    SchemaPin::new(fixture.tool, digest, fixture.semantics)
        .encode()
        .expect("fixture pins encode")
}

#[test]
fn fixture_pins_are_what_tool_provider_computes() {
    for fixture in [DISPATCHED_DEFINITION, CHANGED_DEFINITION] {
        assert_eq!(fixture.tool, SCRIPTED_TOOL);
        assert_eq!(fixture.schema_pin, computed(&fixture), "{fixture:?}");
    }
    // The changed definition is a real change: its pin is another pin.
    assert_ne!(
        DISPATCHED_DEFINITION.schema_pin,
        CHANGED_DEFINITION.schema_pin
    );
}

/// The dispatched fixture carries descriptions, which a pin never covers:
/// the same schema without them has the same pin.
#[test]
fn a_description_edit_keeps_the_fixture_pin() {
    let mut schema = DISPATCHED_DEFINITION.input_schema();
    schema.as_object_mut().unwrap().remove("description");
    schema["properties"]["marker"]
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert_ne!(schema, DISPATCHED_DEFINITION.input_schema());
    let digest = schema_digest(&schema).expect("the bare schema canonicalizes");
    let pin = SchemaPin::new(
        DISPATCHED_DEFINITION.tool,
        digest,
        DISPATCHED_DEFINITION.semantics,
    )
    .encode()
    .expect("the bare pin encodes");
    assert_eq!(pin, DISPATCHED_DEFINITION.schema_pin);
}
