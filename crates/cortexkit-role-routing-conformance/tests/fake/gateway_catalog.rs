use super::*;

fn gateway_catalog() -> Catalog {
    let mut catalog = cortexkit_role_routing_conformance::fixture::catalog();
    catalog.models.truncate(2);
    catalog.models[1].provider = "other-provider".into();
    catalog.accounts.truncate(1);
    catalog.accounts[0].provider = "acme-llm".into();
    catalog.accounts[0].method = "gateway".into();
    catalog.accounts[0].serves = vec![catalog.models[0].provider.clone()];
    catalog
}

fn gateway_routes(catalog: Catalog) -> Vec<String> {
    let state = State {
        gateway_sources: FakeRoute::gateway_sources(&catalog),
        catalog: Some(catalog),
        ..State::default()
    };
    FakeRoute::catalog(&state)
        .models
        .iter()
        .filter(|model| model.provider == "acme-llm")
        .map(CatalogModel::route_key)
        .collect()
}

#[test]
fn gateway_rows_require_gateway_method_and_one_served_provider() {
    let catalog = gateway_catalog();
    assert!(catalog
        .models
        .iter()
        .all(|model| model.provider != "acme-llm"));
    assert_eq!(gateway_routes(catalog.clone()), ["acme-llm/model-a"]);

    for (method, serves) in [
        ("oauth", vec![catalog.models[0].provider.clone()]),
        ("apikey", vec![catalog.models[0].provider.clone()]),
        ("gateway", vec![]),
        (
            "gateway",
            vec![catalog.models[0].provider.clone(), "other-provider".into()],
        ),
    ] {
        let mut catalog = catalog.clone();
        catalog.accounts[0].method = method.into();
        catalog.accounts[0].serves = serves;
        assert!(gateway_routes(catalog).is_empty(), "{method}");
    }
}

#[test]
fn gateway_source_is_shared_across_its_gateway_credentials() {
    let mut catalog = gateway_catalog();
    let mut second = catalog.accounts[0].clone();
    second.account_id = "account-second".into();
    second.credential_id = "credential-second".into();
    second.serves = vec!["other-provider".into()];
    catalog.accounts.push(second);
    assert!(gateway_routes(catalog.clone()).is_empty());

    catalog.accounts[1].serves = catalog.accounts[0].serves.clone();
    assert_eq!(gateway_routes(catalog.clone()), ["acme-llm/model-a"]);

    catalog.accounts[1].method = "oauth".into();
    catalog.accounts[1].serves = vec!["other-provider".into()];
    assert_eq!(gateway_routes(catalog), ["acme-llm/model-a"]);
}
