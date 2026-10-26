//! The configuration the kit sets up before every check, and the ids it
//! uses. Every name is invented; none is a real provider, model or module.

use crate::subject::{
    Catalog, CatalogAccount, CatalogModel, CatalogModule, CatalogRunner, ConsumerConfig,
    ReachableToolProvider,
};
use cortexkit_role_routing::ROLE_DECLARATIONS_KEY;
use serde_json::json;

/// The consumer module most checks select as.
pub const CONSUMER: &str = "example-consumer";
/// A second consumer module, which must not see the first one's decisions.
pub const OTHER_CONSUMER: &str = "other-consumer";
/// The first runner module.
pub const RUNNER_ONE: &str = "runner-one";
/// The second runner module.
pub const RUNNER_TWO: &str = "runner-two";

/// A consumer answered only with routes priced per token.
pub const CODEMODE: &str = "codemode";
/// A consumer answered only with routes priced per token.
pub const FLOW: &str = "flow";
/// A consumer that may be answered with any route; most checks select as
/// it.
pub const SUMMARY: &str = "example_summary";
/// A consumer id the provider is never configured with.
pub const UNCONFIGURED: &str = "example_unconfigured";
/// A name learned only from the declaring module's catalog entry.
pub const DECLARED: &str = "example.declared";
/// The daemon-attested identity declaring the name.
pub const DECLARING_PRINCIPAL: &str = "reserved:example-declarer";
/// An independent module selecting on behalf of scoped work.
pub const DELEGATE_PRINCIPAL: &str = "reserved:example-delegate";

/// The provider whose models are priced per token.
pub const PRICED_PROVIDER: &str = "example-provider";
/// The provider whose models are paid from subscription accounts.
pub const SUBSCRIPTION_PROVIDER: &str = "example-subscription";
/// A second subscription provider, with its own account, so the hold
/// checks can block two ladder entries with different resets.
pub const SECOND_SUBSCRIPTION_PROVIDER: &str = "example-subscription-two";

/// The default program declared by `runner-one`, which also hosts `beta`.
pub const HARNESS: &str = "alpha";

/// The project a check pins a model for.
pub const PINNED_PROJECT: &str = "project-pinned";
/// A project with no pin.
pub const OTHER_PROJECT: &str = "project-other";

/// A tool capability tag `runner-one` can reach, through `example-tools`.
pub const TAG_ONE: &str = "code.read/v1";
/// A tool capability tag `runner-two` can reach, through `other-tools`.
pub const TAG_TWO: &str = "web.search/v1";
/// The tool provider module `runner-one` reaches. It is a module name, not
/// a tag, so a request listing it as a tag is met by no runner.
pub const TOOLS_ONE: &str = "example-tools";
/// The tool provider module `runner-two` reaches.
pub const TOOLS_TWO: &str = "other-tools";

/// The demand every check states unless it needs another; every fixture
/// model meets it.
pub const IQ: u8 = 50;

/// The configured consumers.
pub fn consumers() -> Vec<ConsumerConfig> {
    [(CODEMODE, true), (FLOW, true), (SUMMARY, false)]
        .into_iter()
        .map(|(id, priced_only)| ConsumerConfig {
            id: id.to_owned(),
            priced_only,
            requirements: Default::default(),
        })
        .collect()
}

/// A verified catalog declaration whose output floor only model-c meets.
pub fn catalog_declarations() -> Vec<CatalogModule> {
    vec![CatalogModule {
        principal: DECLARING_PRINCIPAL.into(),
        nonce_verified: true,
        role_declarations: [(
            ROLE_DECLARATIONS_KEY.into(),
            json!({
                "consumers": [{"name": DECLARED, "max_output_tokens": 50_000}]
            }),
        )]
        .into(),
    }]
}

fn model(
    provider: &str,
    model: &str,
    context_window: u64,
    max_output_tokens: u64,
    priced_per_token: bool,
) -> CatalogModel {
    CatalogModel {
        provider: provider.to_owned(),
        model: model.to_owned(),
        iq: 80,
        context_window,
        max_output_tokens,
        tool_calls: true,
        modalities: vec!["image".to_owned()],
        structured_output: true,
        priced_per_token,
    }
}

/// The configured catalog: two priced models, two subscription models, two
/// runners that run every model and reach different tool providers (the
/// first declaring a harness), and two subscription accounts.
pub fn catalog() -> Catalog {
    let models = vec![
        model(PRICED_PROVIDER, "model-a", 200_000, 32_000, true),
        model(PRICED_PROVIDER, "model-b", 128_000, 16_000, true),
        model(SUBSCRIPTION_PROVIDER, "model-c", 200_000, 64_000, false),
        model(SUBSCRIPTION_PROVIDER, "model-d", 100_000, 8_000, false),
    ];
    let route_keys: Vec<String> = models.iter().map(CatalogModel::route_key).collect();
    let account = |n: u32| CatalogAccount {
        provider: SUBSCRIPTION_PROVIDER.to_owned(),
        account_id: format!("account-{n}"),
        credential_id: format!("credential-{n}"),
        method: "oauth".to_owned(),
    };
    Catalog {
        runners: runners(&route_keys),
        models,
        accounts: vec![account(1), account(2)],
    }
}

/// Two runners that run every one of `route_keys` and reach different tool
/// providers; `runner-one` declares [`HARNESS`] as its default program.
fn runners(route_keys: &[String]) -> Vec<CatalogRunner> {
    let runner = |module: &str, tools: &str, tag: &str, harness: Option<&str>| CatalogRunner {
        module: module.to_owned(),
        route_keys: route_keys.to_vec(),
        tool_providers: vec![ReachableToolProvider {
            module: tools.to_owned(),
            capabilities: vec![tag.to_owned()],
        }],
        harnesses: harness.map(|harness| vec![harness.into(), "beta".into()]),
        default_harness: harness.map(str::to_owned),
        starts_sessions: true,
    };
    vec![
        runner(RUNNER_ONE, TOOLS_ONE, TAG_ONE, Some(HARNESS)),
        runner(RUNNER_TWO, TOOLS_TWO, TAG_TWO, None),
    ]
}

/// Programs are runner-wide: two programs with a default, a singleton, and
/// an attach-only gateway preferred ahead of both session-starting runners.
pub fn harness_catalog() -> Catalog {
    let mut catalog = catalog();
    catalog.runners[1].harnesses = Some(vec!["solo".into()]);
    let mut gateway = catalog.runners[0].clone();
    gateway.module = "runner-gateway".into();
    gateway.starts_sessions = false;
    catalog.runners.insert(0, gateway);
    catalog
}

/// The route key of [`hold_catalog`]'s first subscription model.
pub const HOLD_FIRST: &str = "example-subscription/model-c";
/// The route key of [`hold_catalog`]'s second subscription model, on
/// [`SECOND_SUBSCRIPTION_PROVIDER`].
pub const HOLD_SECOND: &str = "example-subscription-two/model-e";
/// The route key of [`hold_catalog`]'s priced model, which no account
/// blocks.
pub const HOLD_PRICED: &str = "example-provider/model-a";

/// The catalog the hold checks configure instead of [`catalog`], in this
/// order: [`HOLD_FIRST`] with accounts `account-1` and `account-2`,
/// [`HOLD_SECOND`] with `account-3`, and [`HOLD_PRICED`].
pub fn hold_catalog() -> Catalog {
    let models = vec![
        model(SUBSCRIPTION_PROVIDER, "model-c", 200_000, 64_000, false),
        model(
            SECOND_SUBSCRIPTION_PROVIDER,
            "model-e",
            200_000,
            64_000,
            false,
        ),
        model(PRICED_PROVIDER, "model-a", 200_000, 32_000, true),
    ];
    let route_keys: Vec<String> = models.iter().map(CatalogModel::route_key).collect();
    let account = |provider: &str, n: u32| CatalogAccount {
        provider: provider.to_owned(),
        account_id: format!("account-{n}"),
        credential_id: format!("credential-{n}"),
        method: "oauth".to_owned(),
    };
    Catalog {
        runners: runners(&route_keys),
        models,
        accounts: vec![
            account(SUBSCRIPTION_PROVIDER, 1),
            account(SUBSCRIPTION_PROVIDER, 2),
            account(SECOND_SUBSCRIPTION_PROVIDER, 3),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_role_routing::declarations::{
        ConsumerDeclarations, RunnerDeclaration, RunnerPair,
    };

    /// Every tool capability tag a fixture runner reaches must be one a real
    /// runner can declare, or a provider loading the fixture refuses it.
    #[test]
    fn every_fixture_tool_tag_passes_the_runner_declaration_validator() {
        for catalog in [catalog(), harness_catalog(), hold_catalog()] {
            for runner in &catalog.runners {
                for provider in &runner.tool_providers {
                    for tag in &provider.capabilities {
                        let mut declaration =
                            RunnerDeclaration::new(vec![RunnerPair::new("example-api", "api-key")]);
                        declaration.tool_capabilities = vec![tag.clone()];
                        let result = ConsumerDeclarations::new()
                            .with_runner(declaration)
                            .validate();
                        assert!(
                            result.is_ok(),
                            "fixture runner {} declares {tag}, which the routing contract refuses: {result:?}",
                            runner.module
                        );
                    }
                }
            }
        }
    }
}
