use super::*;
fn turn(id: &str, status: TurnStatus) -> Turn {
    Turn::Known {
        id: id.into(),
        status,
    }
}
fn reading(activity: Activity, turn: Turn) -> Reading {
    Reading { activity, turn }
}
fn managed(a: Activity, t: Turn) -> Evidence {
    let r = reading(a, t);
    Evidence::Managed(r.clone(), r)
}
fn running(id: &str) -> Evidence {
    managed(Activity::Active, turn(id, TurnStatus::Active))
}
fn waiting(id: &str) -> Evidence {
    managed(Activity::Input, turn(id, TurnStatus::Active))
}
fn completed(id: &str) -> Evidence {
    managed(Activity::Idle, turn(id, TurnStatus::Completed))
}
fn store() -> Store {
    Store::new([(Mode::CodexManaged, "test".into())])
}
fn apply(s: &mut Store, seq: u64, owner: &str, partial: bool, rows: Vec<(&str, Evidence)>) {
    let rows = rows
        .into_iter()
        .map(|(id, evidence)| Row {
            id: id.into(),
            evidence,
        })
        .collect();
    s.apply(
        0,
        seq,
        seq,
        seq,
        1,
        Some("0.167.0".into()),
        Ok(Sample {
            owner: owner.into(),
            partial,
            rows,
            unidentified: 0,
        }),
    );
}
#[test]
fn coherent_transitions_and_duplicates() {
    let mut s = store();
    for (seq, e, expected, alert) in [
        (1, running("x"), State::Running, None),
        (2, waiting("x"), State::WaitingInput, Some("input_wait")),
        (3, waiting("x"), State::WaitingInput, None),
        (4, running("x"), State::Running, None),
        (5, completed("x"), State::Idle, Some("completed")),
        (6, completed("x"), State::Idle, None),
        (7, running("x"), State::Unknown, None),
        (8, running("y"), State::Running, None),
        (9, completed("x"), State::Unknown, None),
        (
            10,
            managed(Activity::Idle, turn("y", TurnStatus::Interrupted)),
            State::Idle,
            None,
        ),
    ] {
        apply(&mut s, seq, "owner", false, vec![("one", e)]);
        assert_eq!(s.sources[0].sessions["one"].facts.activity, expected);
        assert_eq!(s.alerts.first().map(|a| a.kind), alert);
    }
}
#[test]
fn non_atomic_and_unavailable_are_not_terminal() {
    let r = reading(Activity::Active, turn("new", TurnStatus::Active));
    for before in [
        reading(Activity::Input, r.turn.clone()),
        reading(Activity::Active, turn("old", TurnStatus::Active)),
    ] {
        assert_eq!(
            reduce(Mode::CodexManaged, &Evidence::Managed(before, r.clone())),
            Facts::unknown()
        );
    }
    assert_eq!(
        reduce(
            Mode::CodexManaged,
            &managed(Activity::Active, turn("old", TurnStatus::Completed))
        ),
        Facts::unknown()
    );
    assert_eq!(
        reduce(
            Mode::CodexManaged,
            &managed(Activity::Idle, Turn::Unavailable)
        ),
        Facts::unknown()
    );
    assert_eq!(
        reduce(Mode::CodexManaged, &managed(Activity::Idle, Turn::Empty)).latest_turn,
        Turn::Empty
    );
    assert_eq!(
        reduce(
            Mode::CodexManaged,
            &managed(Activity::Idle, turn("x", TurnStatus::Unsupported))
        )
        .confidence,
        Confidence::Unsupported
    );
}
#[test]
fn stale_loss_restart_partial_and_scoped_identity() {
    let mut s = store();
    apply(
        &mut s,
        1,
        "a",
        false,
        vec![("one", waiting("x")), ("two", running("y"))],
    );
    apply(&mut s, 2, "a", true, vec![("one", running("x"))]);
    assert_eq!(s.sources[0].sessions["one"].facts, Facts::unknown());
    assert_eq!(s.sources[0].sessions["two"].visibility, "missing");
    apply(&mut s, 3, "a", false, vec![("one", completed("x"))]);
    assert!(s.alerts.is_empty());
    apply(&mut s, 2, "a", false, vec![]);
    assert_eq!(s.sources[0].sequence, 3);
    s.apply(0, 4, 4, 4, 0, None, Err("unavailable"));
    assert_eq!(s.sources[0].sessions["one"].visibility, "offline");
    apply(&mut s, 5, "b", false, vec![("one", running("x"))]);
    assert_eq!(s.sources[0].sessions.len(), 1);
    assert!(s.alerts.is_empty());
    s.expire(STALE_MS + 5, 100);
    assert_eq!(s.sources[0].sessions["one"].visibility, "stale");
    s.expire(RETAIN_MS + 5, 200);
    assert!(s.sources[0].sessions.is_empty());
}
#[test]
fn partial_and_bounded_list_has_no_alerts() {
    let mut s = store();
    let ids: Vec<_> = (0..ROW_LIMIT + 1).map(|i| i.to_string()).collect();
    apply(
        &mut s,
        1,
        "a",
        false,
        ids.iter().map(|id| (id.as_str(), running("x"))).collect(),
    );
    assert_eq!(s.sources[0].sessions.len(), ROW_LIMIT);
    assert_eq!(s.sources[0].coverage, "partial");
    apply(&mut s, 2, "a", false, vec![]);
    assert!(s.sources[0]
        .sessions
        .values()
        .all(|r| r.facts == Facts::unknown()));
}
#[test]
fn passive_discovery_and_claude_do_not_claim_outcomes() {
    assert_eq!(
        reduce(Mode::CodexPassive, &Evidence::Discovery).confidence,
        Confidence::Provisional
    );
    for (a, state) in [
        (Activity::Active, State::Running),
        (Activity::Idle, State::Idle),
        (Activity::Input, State::Unknown),
    ] {
        let f = reduce(Mode::ClaudePassive, &Evidence::Claude(a));
        assert_eq!((f.activity, f.latest_turn), (state, Turn::Unavailable));
    }
    assert_eq!(
        reduce(Mode::CodexManaged, &Evidence::Discovery),
        Facts::unknown()
    );
}
