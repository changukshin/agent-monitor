use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Deserialize)]
pub struct Frame {
    pub schema: u8,
    pub instance: String,
    pub runtime: String,
    pub revision: u64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "sampledAtMs")]
    pub sampled: u64,
    pub sources: Vec<Source>,
    pub alerts: Vec<Alert>,
}
#[derive(Deserialize)]
pub struct Source {
    pub mode: String,
    pub project: String,
    pub owner: Option<String>,
    pub version: Option<String>,
    pub coverage: String,
    pub sequence: u64,
    pub sessions: BTreeMap<String, Session>,
}
#[derive(Deserialize)]
pub struct Session {
    pub activity: String,
    pub confidence: String,
    pub visibility: String,
    pub observed_ms: u64,
    pub changed_ms: u64,
    pub sequence: u64,
    pub latest_turn: Turn,
}
#[derive(Deserialize)]
pub struct Turn {
    kind: String,
    value: Option<Outcome>,
}
#[derive(Deserialize)]
struct Outcome {
    id: String,
    status: String,
}
#[derive(Deserialize)]
pub struct Alert {
    source: usize,
    session: String,
    turn: String,
    kind: String,
}
fn uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn label(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
impl Frame {
    pub fn valid(&self) -> bool {
        self.schema == 1
            && self.runtime == "windows"
            && self.instance.len() == 64
            && self.instance.bytes().all(|b| b.is_ascii_hexdigit())
            && matches!(self.kind.as_str(), "snapshot" | "update")
            && self.sources.len() <= 8
            && self.alerts.len() <= 512
            && self.sources.iter().all(|s| {
                matches!(
                    s.mode.as_str(),
                    "codex_managed" | "codex_passive" | "claude_passive"
                ) && label(&s.project, 64)
                    && label(&s.coverage, 64)
                    && s.owner.as_ref().is_none_or(|o| label(o, 128))
                    && s.version.as_ref().is_none_or(|v| label(v, 32))
                    && s.sessions.len() <= 64
                    && s.sessions.iter().all(|(id, r)| {
                        uuid(id)
                            && matches!(
                                r.visibility.as_str(),
                                "present" | "missing" | "offline" | "stale"
                            )
                            && r.sequence <= s.sequence
                            && matches!(
                                r.latest_turn.kind.as_str(),
                                "unavailable" | "empty" | "known"
                            )
                            && match (&*r.latest_turn.kind, &r.latest_turn.value) {
                                ("known", Some(t)) => uuid(&t.id),
                                ("known", None) => false,
                                (_, v) => v.is_none(),
                            }
                    })
            })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Status {
    Input,
    Running,
    Completed,
    Interrupted,
    Idle,
    Unknown,
}
impl Status {
    pub fn count(self, counts: &mut [usize; 4]) {
        match self {
            Self::Running => counts[0] += 1,
            Self::Input => counts[1] += 1,
            Self::Completed => counts[2] += 1,
            Self::Unknown => counts[3] += 1,
            _ => (),
        }
    }
    pub fn matches_filter(self, filter: isize) -> bool {
        match filter {
            1 => self == Self::Input,
            2 => self == Self::Running,
            3 => self == Self::Unknown,
            _ => true,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Input => "Input needed",
            Self::Running => "Running",
            Self::Completed => "Idle / Completed",
            Self::Interrupted => "Idle / Interrupted",
            Self::Idle => "Idle",
            Self::Unknown => "Unknown",
        }
    }
}
pub struct Row<'a> {
    pub index: usize,
    pub id: &'a str,
    pub source: &'a Source,
    pub data: &'a Session,
    pub status: Status,
    pub confidence: &'static str,
    pub copies: usize,
}
fn assess(s: &Source, r: &Session, time: u64) -> (Status, &'static str) {
    use Status::*;
    if r.visibility != "present"
        || time
            .checked_sub(r.observed_ms)
            .is_none_or(|age| age >= 15000)
        || !matches!(s.coverage.as_str(), "complete" | "partial")
    {
        return (Unknown, "Unavailable");
    }
    if s.mode == "codex_passive" && r.confidence == "provisional" {
        return (Unknown, "Provisional");
    }
    if r.confidence == "unsupported" {
        return (Unknown, "Unsupported");
    }
    if r.confidence != "confirmed" || s.owner.is_none() {
        return (Unknown, "Unknown");
    }
    let outcome = r.latest_turn.value.as_ref().map(|t| t.status.as_str());
    let state = match (
        s.mode.as_str(),
        s.version.as_deref(),
        r.activity.as_str(),
        outcome,
    ) {
        ("claude_passive", Some("2.1.287"), "running", _) => Running,
        ("claude_passive", Some("2.1.287"), "idle", _) => Idle,
        ("codex_managed", Some("0.167.0"), "waiting_input", Some("active")) => Input,
        ("codex_managed", Some("0.167.0"), "running", Some("active")) => Running,
        ("codex_managed", Some("0.167.0"), "idle", Some("completed")) => Completed,
        ("codex_managed", Some("0.167.0"), "idle", Some("interrupted")) => Interrupted,
        ("codex_managed", Some("0.167.0"), "idle", None) if r.latest_turn.kind == "empty" => Idle,
        _ => Unknown,
    };
    (
        state,
        if state == Unknown {
            "Unknown"
        } else {
            "Confirmed"
        },
    )
}
type Key<'a> = (Option<usize>, Option<&'a str>, &'a str);
fn key<'a>(i: usize, s: &'a Source, id: &'a str) -> Key<'a> {
    (
        (s.mode == "claude_passive" || s.owner.is_none()).then_some(i),
        s.owner.as_deref(),
        id,
    )
}
pub fn rows(f: &Frame, time: u64) -> Vec<Row<'_>> {
    let mut map = BTreeMap::<Key, Row>::new();
    for (i, s) in f.sources.iter().enumerate() {
        for (id, r) in &s.sessions {
            let (status, confidence) = assess(s, r, time);
            let mut row = Row {
                index: i,
                id,
                source: s,
                data: r,
                status,
                confidence,
                copies: 1,
            };
            let k = key(i, s, id);
            if let Some(old) = map.get_mut(&k) {
                old.copies += 1;
                let rank = |r: &Row| {
                    (
                        r.confidence == "Confirmed",
                        r.confidence == "Provisional",
                        r.data.observed_ms,
                    )
                };
                if rank(&row) > rank(old) {
                    row.copies = old.copies;
                    *old = row
                }
            } else {
                map.insert(k, row);
            }
        }
    }
    let mut out: Vec<_> = map.into_values().collect();
    out.sort_by_key(|r| (r.status, &r.source.project, r.id));
    out
}
pub struct Notice {
    pub label: &'static str,
    source: usize,
    session: String,
    turn: String,
    owner: String,
    changed: u64,
    status: Status,
    sampled: u64,
}
impl Notice {
    pub fn current(&self, frame: &Frame, time: u64) -> bool {
        let Some(s) = frame.sources.get(self.source) else {
            return false;
        };
        let Some(r) = s.sessions.get(&self.session) else {
            return false;
        };
        time.checked_sub(self.sampled).is_some_and(|n| n < 5000)
            && s.mode == "codex_managed"
            && s.coverage == "complete"
            && s.owner.as_ref() == Some(&self.owner)
            && r.changed_ms == self.changed
            && r.latest_turn
                .value
                .as_ref()
                .is_some_and(|t| t.id == self.turn)
            && assess(s, r, time).0 == self.status
    }
}
#[derive(Default)]
pub struct Feed {
    pub frame: Option<Frame>,
}
impl Feed {
    pub fn accept(&mut self, f: Frame, time: u64, enabled: bool) -> Result<Vec<Notice>, ()> {
        if !f.valid() {
            return Err(());
        }
        let old = self.frame.as_ref();
        if old.is_some_and(|o| o.instance == f.instance && f.revision <= o.revision) {
            return Ok(vec![]);
        }
        let consecutive = old.is_some_and(|o| {
            o.instance == f.instance && o.revision.checked_add(1) == Some(f.revision)
        }) && f.kind == "update";
        if !consecutive && f.kind != "snapshot" {
            return Err(());
        }
        let mut notices = vec![];
        if enabled && consecutive && time.checked_sub(f.sampled).is_some_and(|n| n < 15000) {
            let previous = rows(old.unwrap(), time);
            let current = rows(&f, time);
            let mut seen = BTreeSet::new();
            for a in &f.alerts {
                let Some(s) = f.sources.get(a.source) else {
                    continue;
                };
                let Some(r) = s.sessions.get(&a.session) else {
                    continue;
                };
                if s.mode != "codex_managed"
                    || s.coverage != "complete"
                    || s.sequence != r.sequence
                    || s.owner.is_none()
                    || r.latest_turn.value.as_ref().is_none_or(|t| t.id != a.turn)
                {
                    continue;
                }
                let status = assess(s, r, time).0;
                let expected = match a.kind.as_str() {
                    "input_wait" if status == Status::Input => "Input needed",
                    "completed" if status == Status::Completed => "Turn completed",
                    _ => continue,
                };
                let k = key(a.source, s, &a.session);
                let matches = |row: &&Row| {
                    key(row.index, row.source, row.id) == k
                        && row
                            .data
                            .latest_turn
                            .value
                            .as_ref()
                            .is_some_and(|t| t.id == a.turn)
                };
                if current
                    .iter()
                    .find(matches)
                    .is_none_or(|row| row.status != status)
                    || previous.iter().find(matches).is_none_or(|row| {
                        !(row.status == Status::Running
                            || (status == Status::Completed && row.status == Status::Input))
                    })
                {
                    continue;
                }
                if seen.insert((k, &a.turn, expected)) {
                    notices.push(Notice {
                        label: expected,
                        source: a.source,
                        session: a.session.clone(),
                        turn: a.turn.clone(),
                        owner: s.owner.clone().unwrap(),
                        changed: r.changed_ms,
                        status,
                        sampled: f.sampled,
                    })
                }
            }
        }
        self.frame = Some(f);
        Ok(notices)
    }
}

#[cfg(test)]
mod tests;
