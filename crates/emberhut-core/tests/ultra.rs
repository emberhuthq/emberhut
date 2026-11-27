//! Ultra runs' rules: the sizes' estimate, a token count in words, a sub-agent's kind, the
//! parts done, and a line for a phone.

use emberhut_core::ultra::{self, AgentKind, Limits, Part, Seen, Size};

fn part(n: f64, size: Size, verify: bool) -> Part {
    Part {
        n,
        name: format!("Part {n}"),
        size,
        verify,
    }
}

#[test]
fn a_plan_is_estimated_from_its_sizes() {
    let plan = [part(1.0, Size::S, true), part(2.0, Size::L, false)];
    let e = ultra::estimate(&plan);
    assert_eq!(e.agents, 3);
    assert!((e.tokens - (120_000.0 + 500_000.0)).abs() < 1e-6);
    let limits = Limits::default();
    assert!(!ultra::big(3.0, e.tokens, &limits));
    assert!(ultra::big(13.0, 1.0, &limits));
    assert!(ultra::big(1.0, 1_600_000.0, &limits));
}

#[test]
fn tokens_read_in_words() {
    assert_eq!(ultra::tokens(1_500_000.0), "1.5M");
    assert_eq!(ultra::tokens(3_000_000.0), "3M");
    assert_eq!(ultra::tokens(1_250_000.0), "1.3M", "a tie goes up");
    assert_eq!(ultra::tokens(200_000.0), "200k");
    assert_eq!(ultra::tokens(1_499.0), "1k");
    assert_eq!(ultra::tokens(800.0), "800");
}

#[test]
fn a_sub_agent_s_kind_and_part_come_from_its_name() {
    assert_eq!(
        ultra::kind_of("Part 3: the parser"),
        (AgentKind::Part, Some(3.0))
    );
    assert_eq!(ultra::kind_of("verify:3"), (AgentKind::Verify, Some(3.0)));
    assert_eq!(
        ultra::kind_of("Review 2 - docs"),
        (AgentKind::Verify, Some(2.0))
    );
    assert_eq!(
        ultra::kind_of("Lane 4: tests"),
        (AgentKind::Lane, Some(4.0))
    );
    assert_eq!(ultra::kind_of("Explore the code"), (AgentKind::Other, None));
}

#[test]
fn a_part_is_done_with_its_check() {
    let plan = [part(1.0, Size::S, true), part(2.0, Size::S, false)];
    let seen = |kind, part, done| Seen { kind, part, done };
    let agents = [
        seen(AgentKind::Part, Some(1.0), true),
        seen(AgentKind::Part, Some(2.0), true),
    ];
    assert_eq!(
        ultra::done(Some(&plan), &agents),
        (1, 2),
        "part 1 waits for its check"
    );
    let checked = [
        agents[0],
        agents[1],
        seen(AgentKind::Verify, Some(1.0), true),
    ];
    assert_eq!(ultra::done(Some(&plan), &checked), (2, 2));
    assert_eq!(
        ultra::done(None, &agents),
        (2, 2),
        "no plan: the part agents seen"
    );
    assert_eq!(
        ultra::done(None, &[seen(AgentKind::Other, None, true)]),
        (1, 1)
    );
}

#[test]
fn a_line_for_a_phone_is_plain_words() {
    assert_eq!(
        ultra::clip(
            "[harness: note] **Purpose:** read `src`\n- one\n## Two",
            100
        ),
        "Purpose: read src one Two"
    );
    assert_eq!(ultra::state_of("completed"), Some("done"));
    assert_eq!(ultra::state_of("cancelled"), Some("stopped"));
    assert!(ultra::lead_brief(&Limits::default(), 4, 3_000_000.0).contains("at most 4 at a time"));
}
