use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const ROW_LIMIT: usize = 64;
pub const STALE_MS: u64 = 15_000;
pub const RETAIN_MS: u64 = 120_000;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    CodexManaged,
    CodexPassive,
    ClaudePassive,
}
impl Mode {
    pub fn version(self) -> &'static str {
        match self {
            Self::ClaudePassive => "2.1.287",
            _ => "0.167.0",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Turn {
    Unavailable,
    Empty,
    Known { id: String, status: TurnStatus },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Active,
    Completed,
    Interrupted,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Active,
    Input,
    Unsupported,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub activity: Activity,
    pub turn: Turn,
}
#[derive(Clone, Debug)]
pub enum Evidence {
    Managed(Reading, Reading),
    Claude(Activity),
    Discovery,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub evidence: Evidence,
}
pub struct Sample {
    pub owner: String,
    pub partial: bool,
    pub rows: Vec<Row>,
    pub unidentified: usize,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Idle,
    WaitingInput,
    Unknown,
    Unsupported,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Confirmed,
    Provisional,
    Unknown,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Facts {
    pub activity: State,
    pub confidence: Confidence,
    pub latest_turn: Turn,
}
impl Facts {
    fn unknown() -> Self {
        Self {
            activity: State::Unknown,
            confidence: Confidence::Unknown,
            latest_turn: Turn::Unavailable,
        }
    }
}
fn reduce(mode: Mode, evidence: &Evidence) -> Facts {
    use Activity as A;
    use State as S;
    let (activity, latest_turn) = match (mode, evidence) {
        (Mode::CodexPassive, Evidence::Discovery) => {
            return Facts {
                confidence: Confidence::Provisional,
                ..Facts::unknown()
            }
        }
        (Mode::ClaudePassive, Evidence::Claude(a)) => (
            match a {
                A::Idle => S::Idle,
                A::Active => S::Running,
                _ => S::Unknown,
            },
            Turn::Unavailable,
        ),
        (Mode::CodexManaged, Evidence::Managed(before, after)) if before == after => {
            let state = match (&after.activity, &after.turn) {
                (
                    A::Active,
                    Turn::Known {
                        status: TurnStatus::Active,
                        ..
                    },
                ) => S::Running,
                (
                    A::Input,
                    Turn::Known {
                        status: TurnStatus::Active,
                        ..
                    },
                ) => S::WaitingInput,
                (
                    A::Idle,
                    Turn::Empty
                    | Turn::Known {
                        status: TurnStatus::Completed | TurnStatus::Interrupted,
                        ..
                    },
                ) => S::Idle,
                (A::Unsupported, _)
                | (
                    _,
                    Turn::Known {
                        status: TurnStatus::Unsupported,
                        ..
                    },
                ) => S::Unsupported,
                _ => S::Unknown,
            };
            // Never expose an old terminal turn as a current outcome on contradictory activity.
            let turn = if state == S::Unknown && matches!(after.turn, Turn::Known { .. }) {
                Turn::Unavailable
            } else {
                after.turn.clone()
            };
            (state, turn)
        }
        _ => return Facts::unknown(),
    };
    let confidence = match activity {
        S::Unknown => Confidence::Unknown,
        S::Unsupported => Confidence::Unsupported,
        _ => Confidence::Confirmed,
    };
    Facts {
        activity,
        confidence,
        latest_turn,
    }
}

#[derive(Debug, Serialize)]
pub struct Session {
    #[serde(flatten)]
    pub facts: Facts,
    pub observed_ms: u64,
    pub changed_ms: u64,
    pub sequence: u64,
    #[serde(skip)]
    seen_at: u64,
    #[serde(skip)]
    retired: VecDeque<String>,
    #[serde(skip)]
    last_turn: Option<(String, TurnStatus)>,
}
#[derive(Debug, Serialize)]
pub struct Source {
    pub mode: Mode,
    pub project: String,
    pub version: Option<String>,
    pub owner: Option<String>,
    pub sequence: u64,
    pub coverage: &'static str,
    pub sample_cost_ms: u64,
    pub observed_ms: u64,
    pub unidentified: usize,
    pub sessions: BTreeMap<String, Session>,
}
#[derive(Serialize, Debug)]
pub struct Alert {
    pub source: usize,
    pub session: String,
    pub turn: String,
    pub kind: &'static str,
}
pub struct Store {
    pub sources: Vec<Source>,
    pub revision: u64,
    pub alerts: Vec<Alert>,
}
impl Store {
    pub fn new(config: impl IntoIterator<Item = (Mode, String)>) -> Self {
        Self {
            sources: config
                .into_iter()
                .map(|(mode, project)| Source {
                    mode,
                    project,
                    version: None,
                    owner: None,
                    sequence: 0,
                    coverage: "starting",
                    sample_cost_ms: 0,
                    observed_ms: 0,
                    unidentified: 0,
                    sessions: BTreeMap::new(),
                })
                .collect(),
            revision: 0,
            alerts: vec![],
        }
    }
    fn change(&mut self) {
        self.revision += 1;
        self.alerts.clear();
    }
    pub fn apply(
        &mut self,
        index: usize,
        seq: u64,
        mono: u64,
        wall: u64,
        cost: u64,
        version: Option<String>,
        sample: Result<Sample, &'static str>,
    ) {
        if seq <= self.sources[index].sequence {
            return;
        }
        self.change();
        let src = &mut self.sources[index];
        src.sequence = seq;
        src.sample_cost_ms = cost;
        src.observed_ms = wall;
        src.version = version;
        let sample = match sample {
            Ok(s) if src.version.as_deref() == Some(src.mode.version()) => s,
            error => {
                src.coverage = error.err().unwrap_or("unsupported_version");
                src.unidentified = 0;
                for row in src.sessions.values_mut() {
                    invalidate(row, wall);
                }
                return;
            }
        };
        let same_owner = src.owner.as_deref() == Some(&sample.owner);
        let continuous = same_owner && src.coverage == "complete" && !sample.partial;
        if !same_owner {
            src.sessions.clear();
        }
        src.owner = Some(sample.owner);
        src.unidentified = sample.unidentified;
        let mut present = BTreeSet::new();
        src.coverage = if sample.partial {
            "partial"
        } else {
            "complete"
        };
        for input in sample.rows {
            if !present.insert(input.id.clone()) {
                continue;
            }
            if !src.sessions.contains_key(&input.id) && src.sessions.len() >= ROW_LIMIT {
                src.coverage = "partial";
                continue;
            }
            let row = src
                .sessions
                .entry(input.id.clone())
                .or_insert_with(|| Session {
                    facts: Facts::unknown(),
                    sequence: 0,
                    observed_ms: wall,
                    changed_ms: wall,
                    seen_at: mono,
                    retired: VecDeque::new(),
                    last_turn: None,
                });
            let mut facts = reduce(src.mode, &input.evidence);
            if let Turn::Known { id, status } = &facts.latest_turn {
                let rollback = row.retired.contains(id)
                    || row.last_turn.as_ref().is_some_and(|(old, s)| {
                        old == id
                            && matches!(s, TurnStatus::Completed | TurnStatus::Interrupted)
                            && s != status
                    });
                if rollback {
                    facts = Facts::unknown();
                } else {
                    if let Some((old, _)) = row.last_turn.replace((id.clone(), *status)) {
                        if old != *id {
                            if row.retired.len() == 8 {
                                row.retired.pop_front();
                            }
                            row.retired.push_back(old);
                        }
                    }
                }
            }
            if continuous
                && mono.saturating_sub(row.seen_at) < STALE_MS
                && row.facts.confidence == Confidence::Confirmed
                && facts.confidence == Confidence::Confirmed
            {
                if let (
                    Turn::Known {
                        id: old,
                        status: TurnStatus::Active,
                    },
                    Turn::Known { id, status },
                ) = (&row.facts.latest_turn, &facts.latest_turn)
                {
                    if old == id {
                        let kind = if *status == TurnStatus::Completed {
                            Some("completed")
                        } else if facts.activity == State::WaitingInput
                            && row.facts.activity == State::Running
                        {
                            Some("input_wait")
                        } else {
                            None
                        };
                        if let Some(kind) = kind {
                            self.alerts.push(Alert {
                                source: index,
                                session: input.id.clone(),
                                turn: id.clone(),
                                kind,
                            });
                        }
                    }
                }
            }
            if facts != row.facts {
                row.changed_ms = wall;
            }
            row.facts = facts;
            row.seen_at = mono;
            row.observed_ms = wall;
            row.sequence = seq;
        }
        for (id, row) in &mut src.sessions {
            if !present.contains(id) {
                invalidate(row, wall);
            }
        }
        if src.coverage != "complete" {
            self.alerts.clear();
        }
    }
    pub fn expire(&mut self, mono: u64, wall: u64) {
        let mut changed = false;
        for src in &mut self.sources {
            src.sessions.retain(|_, row| {
                let age = mono.saturating_sub(row.seen_at);
                if age >= RETAIN_MS {
                    changed = true;
                    return false;
                }
                if age >= STALE_MS && row.facts != Facts::unknown() {
                    invalidate(row, wall);
                    changed = true;
                }
                true
            });
        }
        if changed {
            self.change();
        }
    }
}
fn invalidate(row: &mut Session, wall: u64) {
    if row.facts != Facts::unknown() {
        row.changed_ms = wall;
        row.facts = Facts::unknown();
    }
}
