use crate::{RetryPolicy, Safety};
use serde_json::{Value, json};

pub const DEFAULT_ATTEMPTS: u8 = 3;

pub const UNKNOWN_HINT: &str = "Check the resource in Creations before repeating this request.";

/// What a command's requests have done on Roblox so far, which decides what a failure that ends the command means.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Acceptance {
    /// Roblox has accepted no unsafe write of this command, so a failure keeps its code.
    #[default]
    Open,
    /// `resume` repeats this command's GET requests, which read work an earlier command started. Roblox has accepted
    /// no unsafe write of this command, so a failure keeps its code and carries `resume`.
    Resuming { resume: String },
    /// Roblox accepted an unsafe write and nothing names its resource, so a later failure leaves the outcome unknown.
    Accepted,
    /// Roblox accepted an unsafe write and `resume` reads its resource with GET requests only, so a later failure is
    /// pending.
    Identified { resume: String },
}
impl Acceptance {
    /// Roblox answered an unsafe write with success. A resume command set before it reads other work, so it cannot
    /// recover this write.
    pub fn record(&mut self) {
        if matches!(self, Self::Open | Self::Resuming { .. }) {
            *self = Self::Accepted;
        }
    }
    /// Sets the GET-only command that picks the work up, replacing an earlier one.
    pub fn resume(&mut self, command: String) {
        *self = match self {
            Self::Open | Self::Resuming { .. } => Self::Resuming { resume: command },
            Self::Accepted | Self::Identified { .. } => Self::Identified { resume: command },
        };
    }
    /// A remote failure reports that the work reached a terminal state, so it keeps its code in every state.
    pub fn finish<T>(&self, result: Result<T, Failure>) -> Result<T, Failure> {
        result.map_err(|mut failure| {
            if let Self::Resuming { resume } | Self::Identified { resume } = self {
                failure.error["resume"] = json!(resume);
            }
            if failure.code == 6 {
                return failure;
            }
            match self {
                Self::Open | Self::Resuming { .. } => {}
                Self::Accepted => {
                    failure.code = 5;
                    failure.error["kind"] = json!("outcome_unknown");
                    failure
                        .error
                        .as_object_mut()
                        .expect("a failure is an object")
                        .entry("hint")
                        .or_insert(json!(UNKNOWN_HINT));
                }
                Self::Identified { .. } => {
                    failure.code = 8;
                    failure.error["kind"] = json!("pending");
                }
            }
            failure
        })
    }
}
#[derive(Debug)]
pub struct Failure {
    pub code: u8,
    pub error: Value,
}
impl Failure {
    pub fn new(code: u8, kind: &str, message: impl Into<String>) -> Self {
        Self { code, error: json!({"kind":kind,"message":message.into()}) }
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(2, "usage", message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(7, "unavailable", message)
    }
    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(5, "outcome_unknown", message).with("hint", json!(UNKNOWN_HINT))
    }
    pub fn remote(message: impl Into<String>, detail: Value) -> Self {
        Self::new(6, "remote_failure", message).with("detail", detail)
    }
    pub fn with(mut self, field: &str, value: Value) -> Self {
        self.error[field] = value;
        self
    }
    pub fn document(&self) -> Value {
        json!({"error":self.error})
    }
}

pub enum Attempt {
    PageOverflow { can_resize: bool },
    Redirect(String),
    Response { status: u16, retry_after: Option<String>, body: Vec<u8> },
    Transport { sent: bool, message: String },
}
#[derive(Clone, Copy, Default)]
pub struct History {
    pub attempts: u8,
    pub uncertain: bool,
}
pub enum Verdict {
    Redirect(String),
    Success(Vec<u8>),
    Retry { after_seconds: u32, history: History },
    Fail(Failure),
}

pub fn judge_read(attempt: Attempt, history: History) -> Verdict {
    judge(Safety::Read, None, attempt, history)
}
pub fn judge(safety: Safety, retry: Option<&RetryPolicy>, attempt: Attempt, history: History) -> Verdict {
    let unsafe_write = safety == Safety::Unsafe;
    let (failure, retryable, uncertain, retry_after) = match attempt {
        Attempt::PageOverflow { can_resize } => {
            if unsafe_write {
                return Verdict::Fail(Failure::unknown(
                    "the page exceeds the requested limit; the request will not be repeated",
                ));
            }
            if can_resize && history.attempts == 0 {
                return Verdict::Retry { after_seconds: 0, history: History { attempts: 1, uncertain: false } };
            }
            return Verdict::Fail(Failure::usage(
                "the page cannot fit the requested item limit; use a smaller page size or omit --max-items",
            ));
        }
        Attempt::Redirect(location) => return Verdict::Redirect(location),
        Attempt::Response { status, retry_after, body } => {
            if (200..300).contains(&status) {
                return Verdict::Success(body);
            }
            let message = format!("HTTP {status}");
            let detail =
                serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!(String::from_utf8_lossy(&body)));
            let failure = match status {
                401 | 403 => Failure::new(3, "auth", message),
                429 => Failure::unavailable(message),
                400..500 => Failure::new(4, "rejected", message),
                _ if unsafe_write => Failure::unknown(message),
                _ => Failure::unavailable(message),
            }
            .with("status", json!(status))
            .with("detail", detail);
            let retryable = retry.map_or(!unsafe_write && (status == 429 || status >= 500), |policy| {
                policy.statuses.iter().any(|class| class.matches(status))
            });
            (failure, retryable, unsafe_write && status >= 500, retry_after)
        }
        Attempt::Transport { sent, message } => {
            let failure = if sent && unsafe_write { Failure::unknown(message) } else { Failure::unavailable(message) };
            (failure, !unsafe_write, unsafe_write && sent, None)
        }
    };
    let uncertain = history.uncertain || uncertain;
    let requested =
        retry_after.as_deref().filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()));
    let seconds = requested.and_then(|value| value.parse::<u64>().ok());
    let capped = requested.is_some() && seconds.is_none_or(|wait| wait > 300);
    let failure = if capped {
        failure.with("retryAfter", seconds.map_or_else(|| json!(requested), |seconds| json!(seconds)))
    } else {
        failure
    };
    let attempts = retry.map_or(DEFAULT_ATTEMPTS, |policy| policy.attempts);
    if retryable && !capped && history.attempts + 1 < attempts {
        let delay = retry.map_or(if history.attempts == 0 { 1 } else { 4 }, |policy| policy.delay_seconds);
        return Verdict::Retry {
            after_seconds: seconds
                .map_or(delay, |wait| u32::try_from(wait).expect("retry waits are capped at 300 seconds")),
            history: History { attempts: history.attempts + 1, uncertain },
        };
    }
    if uncertain && failure.code != 5 {
        let mut failure = failure;
        failure.code = 5;
        failure.error["kind"] = json!("outcome_unknown");
        failure.error["hint"] = json!(UNKNOWN_HINT);
        Verdict::Fail(failure)
    } else {
        Verdict::Fail(failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClassName, StatusClass};
    fn response(status: u16) -> Attempt {
        Attempt::Response { status, retry_after: None, body: b"reply".to_vec() }
    }
    fn code(verdict: Verdict) -> u8 {
        match verdict {
            Verdict::Fail(f) => f.code,
            _ => panic!("expected a terminal failure"),
        }
    }
    #[test]
    fn recovery_uses_the_latest_explicit_command() {
        let mut acceptance = Acceptance::default();
        acceptance.record();
        acceptance.resume("old command".into());
        acceptance.resume("new command".into());
        let failure = acceptance.finish::<()>(Err(Failure::unavailable("poll failed"))).unwrap_err();
        assert_eq!(failure.code, 8);
        assert_eq!(failure.error["resume"], "new command", "recovery has one explicit owner");
    }
    fn finished(acceptance: &Acceptance, failure: Failure) -> Failure {
        acceptance.finish::<()>(Err(failure)).unwrap_err()
    }
    #[test]
    fn each_state_maps_a_later_failure_as_the_contract_says() {
        let resumed = |state: fn(String) -> Acceptance| state("crucible oc asset upload --resume x".into());
        for (state, code, kind, resume) in [
            (Acceptance::Open, 7, "unavailable", None),
            (
                resumed(|resume| Acceptance::Resuming { resume }),
                7,
                "unavailable",
                Some("crucible oc asset upload --resume x"),
            ),
            (Acceptance::Accepted, 5, "outcome_unknown", None),
            (
                resumed(|resume| Acceptance::Identified { resume }),
                8,
                "pending",
                Some("crucible oc asset upload --resume x"),
            ),
        ] {
            let failure = finished(&state, Failure::unavailable("poll failed"));
            assert_eq!((failure.code, failure.error["kind"].as_str()), (code, Some(kind)), "{state:?}");
            assert_eq!(failure.error["resume"].as_str(), resume, "{state:?}");
            assert_eq!(failure.error.get("hint").is_some(), code == 5, "{state:?}");
            let remote = finished(&state, Failure::remote("task FAILED", json!({})));
            assert_eq!((remote.code, remote.error["resume"].as_str()), (6, resume), "a remote failure is terminal");
        }
    }
    #[test]
    fn acceptance_and_resume_reach_the_states_in_either_order() {
        let mut state = Acceptance::default();
        state.record();
        assert_eq!(state, Acceptance::Accepted);
        state.resume("poll".into());
        assert_eq!(state, Acceptance::Identified { resume: "poll".into() });
        state.record();
        assert_eq!(state, Acceptance::Identified { resume: "poll".into() }, "a second acceptance keeps the resource");
        let mut state = Acceptance::default();
        state.resume("read".into());
        assert_eq!(state, Acceptance::Resuming { resume: "read".into() });
    }
    #[test]
    fn a_resume_set_before_a_write_is_accepted_cannot_recover_that_write() {
        let mut state = Acceptance::default();
        state.resume("crucible oc asset upload --resume earlier".into());
        state.record();
        assert_eq!(state, Acceptance::Accepted, "the flags gave this write the earlier work's resume command");
        let failure = finished(&state, Failure::unavailable("lost"));
        assert_eq!((failure.code, failure.error.get("resume")), (5, None));
    }
    #[test]
    fn the_enum_writes_what_the_flags_wrote_for_every_history_they_both_allow() {
        #[derive(Clone, Copy)]
        enum Step {
            Record,
            Resume(&'static str),
        }
        fn flags(history: &[Step], mut failure: Failure) -> Failure {
            let accepted = history.iter().any(|s| matches!(s, Step::Record));
            let resume = history.iter().rev().find_map(|s| if let Step::Resume(r) = s { Some(*r) } else { None });
            if let Some(resume) = resume {
                failure.error["resume"] = json!(resume);
            }
            if !accepted || failure.code == 6 {
                return failure;
            }
            if resume.is_some() {
                failure.code = 8;
                failure.error["kind"] = json!("pending");
            } else {
                failure.code = 5;
                failure.error["kind"] = json!("outcome_unknown");
                failure.error.as_object_mut().unwrap().entry("hint").or_insert(json!(UNKNOWN_HINT));
            }
            failure
        }
        let failures = || {
            [
                Failure::usage("bad"),
                Failure::new(3, "auth", "HTTP 403"),
                Failure::new(4, "rejected", "HTTP 404"),
                Failure::unknown("reset"),
                Failure::remote("task FAILED", json!({"state": "FAILED"})),
                Failure::unavailable("HTTP 503").with("status", json!(503)),
                Failure::new(8, "pending", "deadline"),
            ]
        };
        let steps = [Step::Record, Step::Resume("a"), Step::Resume("b")];
        let mut histories: Vec<Vec<Step>> = vec![Vec::new()];
        for length in 1..=4 {
            let longer: Vec<Vec<Step>> = histories
                .iter()
                .filter(|h| h.len() == length - 1)
                .flat_map(|h| steps.iter().map(move |&s| [h.as_slice(), &[s]].concat()))
                .collect();
            histories.extend(longer);
        }
        assert_eq!(histories.len(), 121);
        for history in &histories {
            let mut state = Acceptance::default();
            for step in history {
                match *step {
                    Step::Record => state.record(),
                    Step::Resume(command) => state.resume(command.into()),
                }
            }
            let first_record = history.iter().position(|s| matches!(s, Step::Record));
            let last_resume = history.iter().rposition(|s| matches!(s, Step::Resume(_)));
            let forbidden = matches!((first_record, last_resume), (Some(r), Some(c)) if c < r);
            for (old, new) in failures().into_iter().zip(failures()) {
                let new = finished(&state, new);
                if forbidden {
                    assert!(new.error.get("resume").is_none() && matches!(new.code, 5 | 6), "{state:?}");
                } else {
                    let old = flags(history, old);
                    assert_eq!(
                        (old.code, old.document().to_string()),
                        (new.code, new.document().to_string()),
                        "{state:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn success_row() {
        for s in [Safety::Read, Safety::Idempotent, Safety::Unsafe] {
            assert!(matches!(judge(s, None, response(200), History::default()), Verdict::Success(_)));
        }
    }
    #[test]
    fn redirect_row() {
        assert!(
            matches!(judge(Safety::Read, None, Attempt::Redirect("/next".into()),History::default()),Verdict::Redirect(location) if location=="/next")
        );
    }
    #[test]
    fn page_overflow_row() {
        assert!(matches!(
            judge(Safety::Read, None, Attempt::PageOverflow { can_resize: true }, History::default()),
            Verdict::Retry { after_seconds: 0, .. }
        ));
        assert_eq!(
            code(judge(Safety::Unsafe, None, Attempt::PageOverflow { can_resize: true }, History::default())),
            5
        );
        assert_eq!(code(judge(Safety::Read, None, Attempt::PageOverflow { can_resize: false }, History::default())), 2);
    }
    #[test]
    fn auth_row() {
        for status in [401, 403] {
            assert_eq!(code(judge(Safety::Unsafe, None, response(status), History::default())), 3);
        }
    }
    #[test]
    fn rejection_row() {
        assert_eq!(code(judge(Safety::Unsafe, None, response(400), History::default())), 4);
    }
    #[test]
    fn unsafe_429_row() {
        assert_eq!(code(judge(Safety::Unsafe, None, response(429), History::default())), 7);
    }
    #[test]
    fn unsafe_not_sent_row() {
        assert_eq!(
            code(judge(
                Safety::Unsafe,
                None,
                Attempt::Transport { sent: false, message: "refused".into() },
                History::default()
            )),
            7
        );
    }
    #[test]
    fn unsafe_lost_row() {
        assert_eq!(
            code(judge(
                Safety::Unsafe,
                None,
                Attempt::Transport { sent: true, message: "reset".into() },
                History::default()
            )),
            5
        );
    }
    #[test]
    fn unsafe_server_error_row() {
        assert_eq!(code(judge(Safety::Unsafe, None, response(503), History::default())), 5);
    }
    #[test]
    fn read_retry_row() {
        for status in [429, 503] {
            assert!(matches!(
                judge(Safety::Read, None, response(status), History::default()),
                Verdict::Retry { after_seconds: 1, .. }
            ));
        }
    }
    #[test]
    fn read_transport_retry_row() {
        assert!(matches!(
            judge(
                Safety::Read,
                None,
                Attempt::Transport { sent: true, message: "reset".into() },
                History { attempts: 1, uncertain: false }
            ),
            Verdict::Retry { after_seconds: 4, .. }
        ));
    }
    #[test]
    fn exhaustion_row() {
        assert_eq!(code(judge(Safety::Read, None, response(503), History { attempts: 2, uncertain: false })), 7);
    }
    #[test]
    fn retry_after_row() {
        assert!(matches!(
            judge(
                Safety::Read,
                None,
                Attempt::Response { status: 429, retry_after: Some("9".into()), body: vec![] },
                History::default()
            ),
            Verdict::Retry { after_seconds: 9, .. }
        ));
    }
    #[test]
    fn publish_uncertainty_row() {
        let retry = RetryPolicy {
            statuses: vec![StatusClass::Exact(409), StatusClass::Class(ClassName::ServerError)],
            attempts: 4,
            delay_seconds: 60,
        };
        let Verdict::Retry { history, .. } = judge(Safety::Unsafe, Some(&retry), response(503), History::default())
        else {
            panic!("publish retries 5xx")
        };
        assert_eq!(code(judge(Safety::Unsafe, Some(&retry), response(400), history)), 5);
        assert!(matches!(judge(Safety::Unsafe, Some(&retry), response(200), history), Verdict::Success(_)));
        assert_eq!(
            code(judge(Safety::Unsafe, Some(&retry), response(409), History { attempts: 3, uncertain: false })),
            4
        );
    }
}
