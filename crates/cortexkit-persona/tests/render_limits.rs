//! Execution budgets apply to one render, never to the loaded role's next call.

mod common;

use std::num::{NonZeroU64, NonZeroUsize};

use common::*;
use cortexkit_persona::{
    EmbeddedTier, FragmentSources, PersonaError, RenderFailure, RenderLimits, Tier, TrustClass,
    FUEL_LIMIT, RECURSION_LIMIT,
};

#[test]
fn default_limits_and_nonzero_builders_keep_the_other_budget() {
    let defaults = RenderLimits::default();
    assert_eq!(defaults.recursion().get(), RECURSION_LIMIT);
    assert_eq!(defaults.fuel().get(), FUEL_LIMIT);

    let recursion = NonZeroUsize::new(200).unwrap();
    let fuel = NonZeroU64::new(100_000).unwrap();
    let deeper = defaults.with_recursion(recursion);
    assert_eq!(deeper.recursion(), recursion);
    assert_eq!(deeper.fuel(), defaults.fuel());
    let tighter = defaults.with_fuel(fuel);
    assert_eq!(tighter.fuel(), fuel);
    assert_eq!(tighter.recursion(), defaults.recursion());
    assert_eq!(deeper.with_fuel(fuel), tighter.with_recursion(recursion));
    assert_eq!(defaults, RenderLimits::default());
}

#[test]
fn sixteen_nested_includes_need_an_explicit_recursion_budget() {
    const INCLUDES: usize = 16;
    // RECURSION_LIMIT documents one entry unit and ten units per include.
    // Leave another include's worth of headroom above the required 161 units.
    const INCLUDE_COST: usize = 10;
    let required_frames = 1 + INCLUDES * INCLUDE_COST;
    let budget = NonZeroUsize::new(required_frames + INCLUDE_COST).unwrap();
    let fragments = (0..INCLUDES)
        .map(|i| format!("f{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut bundle = EmbeddedTier::new()
        .with_file(
            ROLE,
            "PERSONA.md",
            full(
                &format!("fragments: [{fragments}]\n"),
                "entry>{% include \"f0\" %}",
            ),
        )
        .unwrap();
    for i in 0..INCLUDES {
        let text = if i + 1 < INCLUDES {
            format!("{i}>{{% include \"f{}\" %}}", i + 1)
        } else {
            format!("{i}>end")
        };
        bundle
            .insert(ROLE, &format!("fragments/f{i}.md"), text)
            .unwrap();
    }
    let role = load(&[Tier::embedded("base", TrustClass::Operator, bundle).unwrap()]).unwrap();
    let context = context();
    let supplied = FragmentSources::new();
    assert!(matches!(
        role.render(&context, &supplied),
        Err(PersonaError::Render {
            failure: RenderFailure::RecursionLimit,
            ..
        })
    ));
    let out = role
        .render_with_limits(
            &context,
            &supplied,
            RenderLimits::default().with_recursion(budget),
        )
        .unwrap();
    assert_eq!(
        out.bytes(),
        b"entry>0>1>2>3>4>5>6>7>8>9>10>11>12>13>14>15>end"
    );
}

#[test]
fn explicit_fuel_budget_stops_a_loop_that_default_fuel_allows() {
    let role = load(&[base(&[(
        "PERSONA.md",
        &full("", "{% for i in range(50000) %}x{% endfor %}"),
    )])])
    .unwrap();
    let context = context();
    let supplied = FragmentSources::new();
    assert_eq!(
        role.render(&context, &supplied).unwrap().bytes(),
        b"x".repeat(50_000)
    );
    assert!(matches!(
        role.render_with_limits(
            &context,
            &supplied,
            RenderLimits::default().with_fuel(NonZeroU64::new(100_000).unwrap()),
        ),
        Err(PersonaError::Render {
            failure: RenderFailure::OutOfFuel,
            ..
        })
    ));
}

#[test]
fn renders_of_one_loaded_role_keep_limits_independent_in_both_orders() {
    let role = load(&[base(&[
        (
            "PERSONA.md",
            &full("fragments: [leaf]\n", "{% include \"leaf\" %}"),
        ),
        ("fragments/leaf.md", "ok"),
    ])])
    .unwrap();
    let context = context();
    let supplied = FragmentSources::new();
    for (tight, failure) in [
        (
            RenderLimits::default().with_recursion(NonZeroUsize::new(1).unwrap()),
            RenderFailure::RecursionLimit,
        ),
        (
            RenderLimits::default().with_fuel(NonZeroU64::new(1).unwrap()),
            RenderFailure::OutOfFuel,
        ),
    ] {
        for order in [[true, false], [false, true]] {
            for use_tight in order {
                if use_tight {
                    assert!(matches!(
                        role.render_with_limits(&context, &supplied, tight),
                        Err(PersonaError::Render { failure: actual, .. }) if actual == failure
                    ));
                } else {
                    assert_eq!(role.render(&context, &supplied).unwrap().bytes(), b"ok");
                    assert_eq!(
                        role.render_with_limits(&context, &supplied, RenderLimits::default())
                            .unwrap()
                            .bytes(),
                        b"ok"
                    );
                }
            }
        }
    }
}
