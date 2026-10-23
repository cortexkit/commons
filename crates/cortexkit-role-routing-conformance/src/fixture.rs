//! The configuration the kit sets up before every check, and the ids it
//! uses. Every name is invented; none is a real provider, model or module.

use crate::subject::{
    Catalog, CatalogAccount, CatalogModel, CatalogRunner, ConsumerConfig, ReachableToolProvider,
};

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

/// The provider whose models are priced per token.
pub const PRICED_PROVIDER: &str = "example-provider";
/// The provider whose models are paid from subscription accounts.
pub const SUBSCRIPTION_PROVIDER: &str = "example-subscription";

/// The project a check pins a model for.
pub const PINNED_PROJECT: &str = "project-pinned";
/// A project with no pin.
pub const OTHER_PROJECT: &str = "project-other";

/// A tool capability tag `runner-one` can reach, through `example-tools`.
pub const TAG_ONE: &str = "code.read/v1";
/// A tool capability tag `runner-two` can reach, through `other-tools`.
pub const TAG_TWO: &str = "search.web/v1";
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
        })
        .collect()
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
/// runners that run every model and reach different tool providers, and two
/// subscription accounts.
pub fn catalog() -> Catalog {
    let models = vec![
        model(PRICED_PROVIDER, "model-a", 200_000, 32_000, true),
        model(PRICED_PROVIDER, "model-b", 128_000, 16_000, true),
        model(SUBSCRIPTION_PROVIDER, "model-c", 200_000, 64_000, false),
        model(SUBSCRIPTION_PROVIDER, "model-d", 100_000, 8_000, false),
    ];
    let route_keys: Vec<String> = models.iter().map(CatalogModel::route_key).collect();
    let runner = |module: &str, tools: &str, tag: &str| CatalogRunner {
        module: module.to_owned(),
        route_keys: route_keys.clone(),
        tool_providers: vec![ReachableToolProvider {
            module: tools.to_owned(),
            capabilities: vec![tag.to_owned()],
        }],
    };
    let account = |n: u32| CatalogAccount {
        provider: SUBSCRIPTION_PROVIDER.to_owned(),
        account_id: format!("account-{n}"),
        credential_id: format!("credential-{n}"),
        method: "oauth".to_owned(),
    };
    Catalog {
        models,
        runners: vec![
            runner(RUNNER_ONE, TOOLS_ONE, TAG_ONE),
            runner(RUNNER_TWO, TOOLS_TWO, TAG_TWO),
        ],
        accounts: vec![account(1), account(2)],
    }
}
