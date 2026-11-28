//! Runners paired at run time: the code, the pairing and its answer, and the runners' list.

use emberhut_api::runner::pairing::{
    FREE_NAME_MAX, NAME_MAX, NewRunnerCode, PairRunner, PairedRunner, RunnerCode, RunnerSource,
    RunnersPage, free_name, is_runner_name, is_runner_url,
};
use emberhut_api::wire;

use crate::support::{assert_round_trip, from_value, to_value};

#[test]
fn a_code_and_a_pairing_keep_their_shape() {
    let code: RunnerCode = assert_round_trip("runner_code.json");
    assert_eq!(code.code, "ABCD-EFGH");
    assert_eq!(code.expires_at.as_str(), "2026-11-28T10:10:00.000Z");
    assert_eq!(code.server, "https://emberhut.example.com");
    assert!(code.reachable);
    assert!(
        code.install
            .command
            .starts_with("emberhut-runner install --server ")
    );
    assert!(
        code.install
            .shell
            .contains("/install-runner.sh | sh -s -- --server ")
    );
    assert!(
        code.install
            .powershell
            .contains("/install-runner.ps1 | iex")
    );
    assert!(
        !format!("{code:?}").contains("ABCD"),
        "Debug hides the code"
    );

    let pair: PairRunner = assert_round_trip("runner_pair_request.json");
    assert_eq!(pair.name, "build");
    assert_eq!(pair.place.as_deref(), Some("Build box"));
    assert!(
        !format!("{pair:?}").contains("ABCD"),
        "Debug hides the code"
    );

    let paired: PairedRunner = assert_round_trip("runner_pair_reply.json");
    assert_eq!(paired.name, "build");
    assert!(
        !format!("{paired:?}").contains(&paired.token),
        "Debug hides the token"
    );
}

#[test]
fn a_pairing_may_leave_out_the_place() {
    let body = serde_json::json!({
        "code": "ABCD-EFGH", "name": "build", "url": "http://192.0.2.7:8800",
        "platform": "linux", "version": "1.4.0"
    });
    let pair: PairRunner = from_value(body.clone());
    assert_eq!(pair.place, None);
    let written = to_value(&from_value::<PairRunner>(body));
    assert!(written.get("place").is_none(), "{written}");
}

#[test]
fn a_code_may_be_asked_for_with_no_address() {
    let asked: NewRunnerCode = from_value(serde_json::json!({}));
    assert_eq!(asked, NewRunnerCode::default());
    assert_eq!(to_value(&asked), serde_json::json!({}));
    let asked: NewRunnerCode =
        from_value(serde_json::json!({ "server": "https://emberhut.example.com" }));
    assert_eq!(
        asked.server.as_deref(),
        Some("https://emberhut.example.com")
    );
}

#[test]
fn a_taken_name_gives_the_next_free_one() {
    assert_eq!(free_name("build", |_| false).as_deref(), Some("build"));
    assert_eq!(
        free_name("build", |name| name == "build").as_deref(),
        Some("build-2")
    );
    let long = "x".repeat(NAME_MAX);
    let next = free_name(&long, |name| name == long).unwrap();
    assert_eq!(next, format!("{}-2", "x".repeat(NAME_MAX - 2)));
    assert!(is_runner_name(&next));
    let ninety = free_name("build", |name| name != format!("build-{FREE_NAME_MAX}")).unwrap();
    assert_eq!(ninety, "build-99");
    assert_eq!(
        free_name("build", |_| true),
        None,
        "every name near it taken"
    );
    assert_eq!(
        free_name("build box", |_| false),
        None,
        "not a runner's name"
    );
}

#[test]
fn the_list_says_where_each_runner_comes_from() {
    let page: RunnersPage = assert_round_trip("runners.json");
    let flag = page.runner("runner").unwrap();
    assert_eq!(flag.source, RunnerSource::Flag);
    assert_eq!(flag.source.as_str(), wire::RUNNER_FROM_FLAG);
    assert!(flag.created.is_none() && flag.platform.is_none());
    let paired = page.runner("build").unwrap();
    assert_eq!(paired.source, RunnerSource::Paired);
    assert_eq!(paired.source.as_str(), wire::RUNNER_PAIRED);
    assert_eq!(paired.version.as_deref(), Some("1.4.0"));
    assert!(page.runner("laptop").is_none());
    let later: RunnerSource = from_value(serde_json::json!("discovered"));
    assert!(!later.is_known(), "a newer server's word is kept");
}

#[test]
fn names_and_addresses_are_the_ones_a_runner_can_have() {
    for name in ["build", "Build-2", "a_b", &"x".repeat(40)] {
        assert!(is_runner_name(name), "{name}");
    }
    for name in ["", "build box", "build/2", "bück", "codes", &"x".repeat(41)] {
        assert!(!is_runner_name(name), "{name}");
    }
    for url in [
        "http://192.0.2.7:8800",
        "https://build.example.com/",
        "https://build.example.com/runner",
    ] {
        assert!(is_runner_url(url), "{url}");
    }
    for url in [
        "",
        "build.example.com",
        "http://",
        "https:///path",
        "https://build.example.com/a b",
        "https://build.example.com/#top",
        "https://build.example.com/?a=1",
        &format!("https://{}", "x".repeat(2048)),
    ] {
        assert!(!is_runner_url(url), "{url}");
    }
}
