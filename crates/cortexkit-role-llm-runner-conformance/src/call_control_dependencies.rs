//! Named prerequisite checks also apply before optional provider fixtures exist.
use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script},
};
use cortexkit_role_llm_runner::{describe::RoleDescribe, ops};

fn check(name: &str, describe: &RoleDescribe) -> Result<Ending, String> {
    let (group, dependency) = name
        .split_once("_without_")
        .ok_or("unknown dependency case")?;
    if !describe.declares(group) {
        return Ok(Ending::Inapplicable(format!(
            "runner does not declare {group}"
        )));
    }
    if !describe.declares(dependency) {
        return Err(name.to_owned());
    }
    Ok(Ending::Passed)
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let session = case
        .open("call-control-describe", Script::default())
        .await?;
    let describe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(&session.route, ops::ROLE_DESCRIBE, serde_json::json!({})).await?,
        )?,
    )?;
    check(name, &describe)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faulty_describes_fail_each_dependency_case_by_name() {
        for name in [
            "call_yield_without_plans",
            "call_yield_without_delivery_order",
            "call_yield_without_notices",
            "call_yield_without_message_marks",
            "stop_all_without_run_cancel_by_interrupt",
        ] {
            let (group, dependency) = name.split_once("_without_").unwrap();
            let faithful =
                RoleDescribe::default().with_capabilities(vec![group.into(), dependency.into()]);
            assert!(matches!(check(name, &faithful), Ok(Ending::Passed)));
            let faulty = RoleDescribe::default().with_capabilities(vec![group.into()]);
            assert_eq!(
                check(name, &faulty).err().expect("faulty describe passed"),
                name
            );
        }
    }
}
