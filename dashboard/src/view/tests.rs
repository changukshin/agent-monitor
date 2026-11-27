use super::*;
use serde_json::{json, Value};
const SID: &str = "10000000-0000-0000-0000-000000000001";
const TURN: &str = "20000000-0000-0000-0000-000000000002";
fn source(mode: &str, owner: &str, activity: &str) -> Value {
    json!({"mode":mode,"owner":owner,"project":"Test","version":if mode=="claude_passive"{"2.1.287"}else{"0.167.0"},"coverage":"complete","sequence":1,
"sessions":{(SID):{"activity":activity,"sequence":1,"confidence":if mode=="codex_passive"{"provisional"}else{"confirmed"},"visibility":"present","observed_ms":900,"latest_turn":{"kind":"known","value":{"id":TURN,"status":if activity=="idle"{"completed"}else{"active"}}}}}})
}
fn frame(rev: u64, kind: &str, sources: Vec<Value>) -> Frame {
    serde_json::from_value(json!({"schema":1,"runtime":"windows","instance":"a".repeat(64),"revision":rev,"type":kind,"sampledAtMs":1000,"sources":sources,"alerts":[]})).unwrap()
}
#[test]
fn groups_once_within_owner_and_prefers_valid_managed() {
    let m = source("codex_managed", "one", "waiting_input");
    let p = source("codex_passive", "one", "unknown");
    let f = frame(
        1,
        "snapshot",
        vec![
            p.clone(),
            m.clone(),
            source("codex_managed", "two", "idle"),
            source("claude_passive", "one", "idle"),
        ],
    );
    let r = rows(&f, 1000);
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].copies, 2);
    assert_eq!(r[0].status, Status::Input);
    assert_eq!(r[1].status, Status::Completed);
    assert_eq!(r[2].status, Status::Idle);
    let mut degraded = m.clone();
    degraded["sessions"][SID]["visibility"] = json!("missing");
    let f = frame(2, "update", vec![degraded, p]);
    assert_eq!(rows(&f, 1000)[0].confidence, "Provisional");
    let mut partial = m;
    partial["coverage"] = json!("partial");
    let f = frame(2, "update", vec![partial]);
    assert_eq!(rows(&f, 1000)[0].status, Status::Input);
    assert_eq!(rows(&f, 16000)[0].status, Status::Unknown);
}
fn transition(rev: u64, activity: &str) -> Frame {
    let mut f = frame(
        rev,
        if rev == 1 { "snapshot" } else { "update" },
        vec![source("codex_managed", "one", activity)],
    );
    for _ in 0..2 {
        f.alerts.push(Alert {
            source: 0,
            session: SID.into(),
            turn: TURN.into(),
            kind: if activity == "idle" {
                "completed"
            } else {
                "input_wait"
            }
            .into(),
        });
    }
    f
}
#[test]
fn alerts_require_fresh_consecutive_transition_and_explicit_opt_in() {
    let mut feed = Feed::default();
    for (rev, act, enabled, count) in [
        (1, "running", true, 0),
        (2, "waiting_input", true, 1),
        (2, "waiting_input", true, 0),
        (3, "waiting_input", true, 0),
        (4, "running", true, 0),
        (5, "waiting_input", true, 1),
        (6, "running", false, 0),
        (7, "waiting_input", false, 0),
        (8, "idle", true, 1),
    ] {
        assert_eq!(
            feed.accept(transition(rev, act), 1000, enabled)
                .unwrap()
                .len(),
            count
        );
    }
    let mut gap = transition(11, "idle");
    assert!(feed.accept(transition(11, "idle"), 1000, true).is_err());
    gap.kind = "snapshot".into();
    assert!(feed.accept(gap, 1000, true).unwrap().is_empty());
    let mut replacement = frame(0, "snapshot", vec![]);
    replacement.instance = "b".repeat(64);
    assert!(feed.accept(replacement, 1000, true).unwrap().is_empty());
    assert!(rows(feed.frame.as_ref().unwrap(), 1000).is_empty());
}
#[test]
fn malformed_scope_and_unsupported_alerts_are_suppressed() {
    for case in 0..6 {
        let mut feed = Feed::default();
        feed.accept(transition(1, "running"), 1000, true).unwrap();
        let mut f = transition(2, "waiting_input");
        let s = &mut f.sources[0];
        match case {
            0 => s.coverage = "partial".into(),
            1 => s.sessions.get_mut(SID).unwrap().confidence = "unsupported".into(),
            2 => s.owner = None,
            3 => s.mode = "claude_passive".into(),
            4 => f
                .alerts
                .iter_mut()
                .for_each(|a| a.turn = "different".into()),
            _ => f.sampled = 0,
        }
        assert!(feed
            .accept(f, if case == 5 { 20000 } else { 1000 }, true)
            .unwrap()
            .is_empty());
    }
    let mut f = frame(1, "snapshot", vec![]);
    f.schema = 2;
    assert!(!f.valid());
    let f = frame(
        1,
        "snapshot",
        vec![source("codex_managed", "one", "running"); 9],
    );
    assert!(!f.valid());
}

#[test]
fn grouped_summary_and_attention_filter_share_rows() {
    let f = frame(
        1,
        "snapshot",
        vec![
            source("codex_passive", "one", "unknown"),
            source("codex_managed", "one", "waiting_input"),
            source("claude_passive", "two", "running"),
        ],
    );
    let list = rows(&f, 1000);
    let mut counts = [0; 4];
    for row in &list {
        row.status.count(&mut counts);
    }
    assert_eq!(counts, [1, 1, 0, 0]);
    assert_eq!(
        list.iter().filter(|r| r.status.matches_filter(1)).count(),
        1
    );
}
