use crate::auth::Secret;
use crate::outcome::{Attempt, DEFAULT_ATTEMPTS, Failure, History, Verdict, judge, judge_read};
use crate::{Auth, Payload, RetryPolicy, Safety};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use ureq::RequestExt;

pub enum Event<'a> {
    Note(&'a str),
    Trace(&'a str),
    /// Roblox answered an unsafe write with success, so a later failure leaves the overall outcome unknown.
    Accepted,
}
pub trait Sink {
    fn event(&self, event: Event<'_>);
}
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
pub trait Request {
    fn operation(&self) -> Operation<'_>;
    fn build(&self, origin: &str) -> Result<ureq::http::Request<Vec<u8>>, Failure>;
    fn until(&self) -> Option<Instant>;
}
pub struct Operation<'a> {
    pub origin: &'static str,
    pub path: &'a str,
    pub auth: &'a Auth,
    pub safety: Safety,
    pub retry: Option<&'a RetryPolicy>,
    pub response: Payload<()>,
    pub timeout: Duration,
}
pub struct Transport<'a> {
    agent: ureq::Agent,
    downloads: Downloads,
    origin: Option<String>,
    key: Option<&'a Secret>,
    pub timing: Timing,
    sink: Box<dyn Sink + 'a>,
    clock: Box<dyn Clock + 'a>,
}
#[derive(Clone, Copy)]
pub struct Timing {
    pub poll_interval: Duration,
    pub deadline: Duration,
    pub retry_delay: Option<Duration>,
}
pub enum Reply {
    Json(Value),
    Empty,
}
impl Reply {
    pub fn json(self) -> Result<Value, Failure> {
        match self {
            Self::Json(v) => Ok(v),
            Self::Empty => Ok(Value::Null),
        }
    }
}
impl<'a> Transport<'a> {
    pub fn new(
        origin: Option<String>,
        key: Option<&'a Secret>,
        timing: Timing,
        proxy: Option<ureq::Proxy>,
        sink: Box<dyn Sink + 'a>,
        clock: Box<dyn Clock + 'a>,
    ) -> Result<Self, Failure> {
        if let Some(origin) = &origin {
            let url =
                url::Url::parse(origin).map_err(|_| Failure::usage("CRUCIBLE_OC_ORIGIN must be an HTTP origin"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(Failure::usage("CRUCIBLE_OC_ORIGIN must be an HTTP origin"));
            }
            sink.event(Event::Note("Warning: CRUCIBLE_OC_ORIGIN overrides Roblox servers."));
        }
        let agent = ureq::Agent::config_builder()
            .proxy(proxy.clone())
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .into();
        let downloads = Downloads {
            agent: ureq::Agent::config_builder()
                .proxy(proxy)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(30)))
                .timeout_recv_response(Some(Duration::from_secs(30)))
                .timeout_per_read(Some(Duration::from_secs(30)))
                .timeout_global(None)
                .build()
                .into(),
            allow_loopback: origin.is_some(),
            retry_delay: timing.retry_delay,
            retry_budget: timing.deadline,
        };
        Ok(Self { agent, downloads, origin, key, timing, sink, clock })
    }
    pub fn clock(&self) -> &dyn Clock {
        &*self.clock
    }
    pub fn send(&self, call: &impl Request) -> Result<Reply, Failure> {
        self.send_noting(call, &mut false)
    }
    /// `send`, which also sets `acted` when an attempt may have changed something on Roblox: it got a 2xx or a 5xx, or
    /// its answer was lost after the request left.
    pub(crate) fn send_noting(&self, call: &impl Request, acted: &mut bool) -> Result<Reply, Failure> {
        let op = call.operation();
        let remaining = || call.until().map(|until| until.saturating_duration_since(self.clock.now()));
        let mut history = History::default();
        loop {
            if remaining().is_some_and(|remaining| remaining.is_zero()) {
                return Err(Failure::unavailable("request deadline passed"));
            }
            let mut request = call.build(self.origin.as_deref().unwrap_or(op.origin))?;
            if matches!(op.auth, Auth::ApiKey { .. }) {
                let key = self.key.ok_or_else(|| Failure::new(3, "auth", "no API key selected"))?;
                request.headers_mut().insert(
                    "x-api-key",
                    ureq::http::HeaderValue::from_str(key.expose())
                        .map_err(|_| Failure::new(3, "auth", "selected key is not a valid header value"))?,
                );
            }
            if let Auth::BodyKey { field } = &op.auth {
                let key = self.key.ok_or_else(|| Failure::new(3, "auth", "no API key selected"))?;
                *request.body_mut() =
                    serde_json::to_vec(&json!({field:key.expose()})).expect("a JSON value serializes");
                request.headers_mut().insert("content-type", ureq::http::HeaderValue::from_static("application/json"));
            }
            self.sink.event(Event::Trace(&format!("{} {}", request.method(), op.path)));
            let timeout = remaining().map_or(op.timeout, |remaining| remaining.min(op.timeout));
            let attempt = match request.with_agent(&self.agent).configure().timeout_global(Some(timeout)).run() {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    if (200..300).contains(&status) && op.safety == Safety::Unsafe {
                        self.sink.event(Event::Accepted);
                    }
                    self.sink.event(Event::Trace(&format!("HTTP {status}")));
                    let retry_after = response
                        .headers()
                        .get("retry-after")
                        .and_then(|s| s.to_str().ok())
                        .map(|s| s.trim().to_owned());
                    match response.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec() {
                        Ok(body) => {
                            if (200..300).contains(&status)
                                && matches!(op.response, Payload::Json { .. })
                                && status != 204
                                && !body.is_empty()
                                && serde_json::from_slice::<Value>(&body).is_err()
                            {
                                if op.safety == Safety::Unsafe {
                                    return Err(Failure::unknown("invalid JSON response")
                                        .with("detail", json!(String::from_utf8_lossy(&body))));
                                }
                                Attempt::Transport {
                                    sent: true,
                                    message: format!("invalid JSON: {}", String::from_utf8_lossy(&body)),
                                }
                            } else {
                                Attempt::Response { status, retry_after, body }
                            }
                        }
                        Err(error) if !(200..300).contains(&status) => {
                            Attempt::Response { status, retry_after, body: error.to_string().into_bytes() }
                        }
                        Err(error) => Attempt::Transport { sent: true, message: error.to_string() },
                    }
                }
                Err(error) => {
                    let not_sent = matches!(
                        error,
                        ureq::Error::HostNotFound
                            | ureq::Error::ConnectionFailed
                            | ureq::Error::Timeout(ureq::Timeout::Connect | ureq::Timeout::Resolve)
                    ) || matches!(&error,ureq::Error::Io(e) if matches!(e.kind(),std::io::ErrorKind::ConnectionRefused|std::io::ErrorKind::AddrNotAvailable|std::io::ErrorKind::NetworkUnreachable|std::io::ErrorKind::HostUnreachable));
                    Attempt::Transport { sent: !not_sent, message: error.to_string() }
                }
            };
            *acted |= match &attempt {
                Attempt::Response { status, .. } => (200..300).contains(status) || *status >= 500,
                Attempt::Transport { sent, .. } => *sent,
                Attempt::Redirect(_) | Attempt::PageOverflow { .. } => false,
            };
            let reason = match &attempt {
                Attempt::Response { status, .. } => format!("HTTP {status}"),
                Attempt::Transport { .. } => "request failed".into(),
                Attempt::Redirect(_) => "redirect".into(),
                Attempt::PageOverflow { .. } => "page exceeds limit".into(),
            };
            match judge(op.safety, op.retry, attempt, history) {
                Verdict::Redirect(_) => unreachable!("API requests do not follow redirects"),
                Verdict::Success(body) => {
                    if body.is_empty() {
                        return Ok(Reply::Empty);
                    }
                    return match &op.response {
                        Payload::Json { .. } if !body.is_empty() => serde_json::from_slice(&body)
                            .map(Reply::Json)
                            .map_err(|_| Failure::unavailable("invalid JSON response")),
                        Payload::Json { .. } | Payload::Empty => Ok(Reply::Empty),
                        Payload::Text | Payload::Binary => {
                            Err(Failure::unavailable("unsupported registry response payload"))
                        }
                    };
                }
                Verdict::Retry { after_seconds, history: next } => {
                    history = next;
                    let delay = self.timing.retry_delay.unwrap_or(Duration::from_secs(after_seconds.into()));
                    let attempts = op.retry.map_or(DEFAULT_ATTEMPTS, |r| r.attempts);
                    self.sink.event(Event::Note(&format!(
                        "{reason}, retrying in {} s (attempt {} of {attempts})",
                        delay.as_secs_f64(),
                        history.attempts + 1
                    )));
                    self.clock.sleep(remaining().map_or(delay, |remaining| delay.min(remaining)));
                }
                Verdict::Fail(failure) => return Err(failure),
            }
        }
    }
    pub fn download(&self, location: &str, to: &std::path::Path, overwrite: bool) -> Result<u64, Failure> {
        self.downloads.save(location, to, overwrite, &*self.sink, &*self.clock)
    }
}
struct Downloads {
    agent: ureq::Agent,
    allow_loopback: bool,
    retry_delay: Option<Duration>,
    retry_budget: Duration,
}
impl Downloads {
    fn save(
        &self,
        location: &str,
        to: &std::path::Path,
        overwrite: bool,
        sink: &dyn Sink,
        clock: &dyn Clock,
    ) -> Result<u64, Failure> {
        use std::io::Seek;
        let retry_until = clock.now() + self.retry_budget;
        let mut url = url::Url::parse(location).map_err(|_| Failure::unavailable("delivery location is not a URL"))?;
        let parent = to.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."));
        if to.exists() && !overwrite {
            return Err(Failure::usage("output already exists; use --overwrite"));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| Failure::usage("cannot create a temporary output file"))?;
        for redirects in 0..=5 {
            let test_http = self.allow_loopback
                && url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
            if (url.scheme() != "https" && !test_http) || !url.username().is_empty() || url.password().is_some() {
                return Err(Failure::unavailable("download requires HTTPS without URL credentials"));
            }
            url.set_fragment(None);
            let mut history = History::default();
            loop {
                temporary
                    .as_file_mut()
                    .set_len(0)
                    .and_then(|()| temporary.rewind())
                    .map_err(|_| Failure::unavailable("cannot reset the temporary download file"))?;
                let mut saved = 0;
                let attempt = match self.agent.get(url.as_str()).call() {
                    Ok(mut response) => {
                        let status = response.status().as_u16();
                        let retry_after = response
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .map(|v| v.trim().to_owned());
                        if (200..300).contains(&status) {
                            match std::io::copy(&mut response.body_mut().as_reader(), temporary.as_file_mut()) {
                                Ok(bytes) => {
                                    saved = bytes;
                                    Attempt::Response { status, retry_after, body: vec![] }
                                }
                                Err(_) => Attempt::Transport {
                                    sent: true,
                                    message: "download body was lost; the output was not replaced".into(),
                                },
                            }
                        } else if matches!(status, 301 | 302 | 303 | 307 | 308) {
                            Attempt::Redirect(
                                response
                                    .headers()
                                    .get("location")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or_default()
                                    .into(),
                            )
                        } else {
                            let body = response
                                .body_mut()
                                .with_config()
                                .limit(64 * 1024 * 1024)
                                .read_to_vec()
                                .unwrap_or_default();
                            Attempt::Response { status, retry_after, body }
                        }
                    }
                    Err(error) => Attempt::Transport { sent: true, message: error.to_string() },
                };
                match judge_read(attempt, history) {
                    Verdict::Success(_) => {
                        temporary.as_file().sync_all().map_err(|_| Failure::usage("cannot flush the output file"))?;
                        let result = if overwrite { temporary.persist(to) } else { temporary.persist_noclobber(to) };
                        result.map_err(|_| Failure::usage("cannot rename the temporary output file"))?;
                        return Ok(saved);
                    }
                    Verdict::Redirect(location) => {
                        if redirects == 5 {
                            return Err(Failure::unavailable("download exceeded five redirects"));
                        }
                        if location.is_empty() {
                            return Err(Failure::unavailable("download redirect has no location"));
                        }
                        url = url
                            .join(&location)
                            .map_err(|_| Failure::unavailable("download redirect has an invalid URL"))?;
                        break;
                    }
                    Verdict::Retry { after_seconds, history: next } => {
                        history = next;
                        let delay = self.retry_delay.unwrap_or(Duration::from_secs(after_seconds.into()));
                        sink.event(Event::Note(&format!(
                            "download request failed, retrying in {} s (attempt {} of {DEFAULT_ATTEMPTS})",
                            delay.as_secs_f64(),
                            history.attempts + 1
                        )));
                        let remaining = retry_until.saturating_duration_since(clock.now());
                        clock.sleep(delay.min(remaining));
                        if clock.now() >= retry_until {
                            return Err(Failure::unavailable("download retry deadline passed"));
                        }
                    }
                    Verdict::Fail(mut failure) => {
                        if failure.code == 3 {
                            failure.code = 4;
                            failure.error["kind"] = json!("rejected");
                        }
                        return Err(failure);
                    }
                }
            }
        }
        unreachable!("the redirect limit returns within the loop")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Recorder {
        notes: RefCell<Vec<String>>,
        slept: RefCell<Vec<Duration>>,
    }
    impl Sink for &Recorder {
        fn event(&self, event: Event<'_>) {
            if let Event::Note(line) = event {
                self.notes.borrow_mut().push(line.into());
            }
        }
    }
    impl Clock for &Recorder {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn sleep(&self, duration: Duration) {
            self.slept.borrow_mut().push(duration);
        }
    }
    struct Get(Auth);
    impl Request for Get {
        fn operation(&self) -> Operation<'_> {
            Operation {
                origin: "https://apis.roblox.com",
                path: "/thing",
                auth: &self.0,
                safety: Safety::Read,
                retry: None,
                response: Payload::Empty,
                timeout: Duration::from_secs(30),
            }
        }
        fn build(&self, origin: &str) -> Result<ureq::http::Request<Vec<u8>>, Failure> {
            Ok(ureq::http::Request::get(format!("{origin}/thing")).body(vec![]).unwrap())
        }
        fn until(&self) -> Option<Instant> {
            None
        }
    }
    fn timing() -> Timing {
        Timing { deadline: Duration::from_secs(300), poll_interval: Duration::ZERO, retry_delay: None }
    }

    #[test]
    fn retries_sleep_on_the_injected_clock_and_report_to_the_sink() {
        let server = httpmock::MockServer::start();
        let m = server.mock(|_, t| {
            t.status(503);
        });
        let recorder = Recorder::default();
        let transport =
            Transport::new(Some(server.base_url()), None, timing(), None, Box::new(&recorder), Box::new(&recorder))
                .unwrap();
        let failure = transport.send(&Get(Auth::None)).err().unwrap();
        assert_eq!(failure.code, 7);
        m.assert_calls(3);
        assert_eq!(*recorder.slept.borrow(), [Duration::from_secs(1), Duration::from_secs(4)]);
        assert_eq!(
            *recorder.notes.borrow(),
            [
                "Warning: CRUCIBLE_OC_ORIGIN overrides Roblox servers.",
                "HTTP 503, retrying in 1 s (attempt 2 of 3)",
                "HTTP 503, retrying in 4 s (attempt 3 of 3)",
            ]
        );
    }

    #[test]
    fn agents_use_only_the_proxy_they_are_given() {
        let recorder = Recorder::default();
        let proxy = ureq::Proxy::new("http://127.0.0.1:9").unwrap();
        let transport =
            Transport::new(None, None, timing(), Some(proxy), Box::new(&recorder), Box::new(SystemClock)).unwrap();
        assert!(transport.agent.config().proxy().is_some());
        assert!(transport.downloads.agent.config().proxy().is_some());
        let transport = Transport::new(None, None, timing(), None, Box::new(&recorder), Box::new(SystemClock)).unwrap();
        assert!(transport.agent.config().proxy().is_none());
        assert!(transport.downloads.agent.config().proxy().is_none());
    }
    #[test]
    fn downloads_have_idle_budgets_and_api_calls_have_ten_minutes() {
        let recorder = Recorder::default();
        let transport = Transport::new(None, None, timing(), None, Box::new(&recorder), Box::new(SystemClock)).unwrap();
        assert!(
            transport.downloads.agent.config().timeouts().global.is_none(),
            "active downloads have no total duration cap"
        );
        assert_eq!(
            transport.downloads.agent.config().timeout_per_read(),
            Some(Duration::from_secs(30)),
            "a stalled read still times out"
        );
        assert_eq!(
            transport.agent.config().timeouts().global,
            Some(Duration::from_secs(600)),
            "binary transfers can run for ten minutes"
        );
    }
}
