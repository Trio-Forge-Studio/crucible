use crate::call::Call;
use crate::output::Output;
use crucible_oc::auth::Secret;
use crucible_oc::outcome::{Acceptance, Attempt, Failure, History, Verdict, judge};
use crucible_oc::transport::{Event, Operation, Reply, Request, Sink, SystemClock, Timing, Transport};
use oc_registry::{Completion, Method, Op, Registry};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::Duration;

pub struct Executor<'a> {
    pub registry: &'static Registry,
    acceptance: &'a RefCell<Acceptance>,
    transport: Transport<'a>,
    out: &'a Output,
}
struct Events<'a> {
    out: &'a Output,
    acceptance: &'a RefCell<Acceptance>,
}
impl Sink for Events<'_> {
    fn event(&self, event: Event<'_>) {
        match event {
            Event::Note(line) => self.out.note(line),
            Event::Trace(line) => self.out.trace(line),
            Event::Accepted => self.acceptance.borrow_mut().record(),
        }
    }
}
impl Request for Call<'_> {
    fn operation(&self) -> Operation<'_> {
        let op = self.op();
        Operation {
            origin: op.server.origin(),
            path: &op.path,
            auth: &op.auth,
            safety: op.safety,
            retry: op.retry.as_ref(),
            response: op.response.kind(),
            timeout: request_timeout(self),
        }
    }
    fn build(&self, origin: &str) -> Result<ureq::http::Request<Vec<u8>>, Failure> {
        self.request(origin)
    }
    fn until(&self) -> Option<std::time::Instant> {
        self.deadline()
    }
}
impl<'a> Executor<'a> {
    pub fn new(
        registry: &'static Registry,
        origin: Option<String>,
        key: Option<&'a Secret>,
        timing: Timing,
        out: &'a Output,
        acceptance: &'a RefCell<Acceptance>,
    ) -> Result<Self, Failure> {
        let events = Events { out, acceptance };
        let transport = Transport::new(origin, key, timing, Box::new(events), Box::new(SystemClock))?;
        Ok(Self { registry, acceptance, transport, out })
    }
    pub fn send(&self, call: &Call<'_>) -> Result<Reply, Failure> {
        self.transport.send(call)
    }
    pub fn send_all(&self, call: Call<'_>, max_items: Option<usize>, max_pages: u32) -> Result<Value, Failure> {
        let p = call.op().pagination.as_ref().ok_or_else(|| Failure::usage("operation does not declare pagination"))?;
        if max_items == Some(0) {
            return Err(Failure::usage("--max-items must be positive"));
        }
        let mut call = if let Some(limit) = max_items.filter(|_| call.page_size().is_some()) {
            call.limit_page(limit)?
        } else {
            call
        };
        let mut result = Value::Null;
        let mut items = vec![];
        let mut tokens = BTreeSet::new();
        let mut resize_history = History::default();
        let mut pages = 0;
        loop {
            let page = self.send(&call)?.json()?;
            let page_items = page[&p.items_field]
                .as_array()
                .ok_or_else(|| Failure::unavailable(format!("page has no {} array", p.items_field)))?;
            if let Some(remaining) =
                max_items.map(|limit| limit - items.len()).filter(|remaining| page_items.len() > *remaining)
            {
                let can_resize = p.size_param.is_some() && call.page_size().is_none_or(|size| size > remaining);
                match judge(
                    call.op().safety,
                    call.op().retry.as_ref(),
                    Attempt::PageOverflow { can_resize },
                    resize_history,
                ) {
                    Verdict::Retry { after_seconds, history } => {
                        resize_history = history;
                        self.transport.clock().sleep(
                            self.transport.timing.retry_delay.unwrap_or(Duration::from_secs(after_seconds.into())),
                        );
                        call = call.limit_page(remaining)?;
                        continue;
                    }
                    Verdict::Fail(failure) => return Err(failure.with("detail", page)),
                    _ => unreachable!("page overflow returns retry or failure"),
                }
            }
            pages += 1;
            items.extend(page_items.iter().cloned());
            if result.is_null() {
                result = page.clone();
            }
            let token = page[&p.next_field].as_str().unwrap_or_default();
            result[&p.next_field] = json!(token);
            if token.is_empty() || pages >= max_pages || max_items.is_some_and(|limit| items.len() >= limit) {
                break;
            }
            if !tokens.insert(token.to_owned()) {
                return Err(Failure::unavailable("server repeated a pagination token"));
            }
            call = call.next_page(
                token,
                max_items.map(|limit| limit - items.len()).filter(|remaining| *remaining < page_items.len()),
            )?;
            resize_history = History::default();
        }
        result[&p.items_field] = json!(items);
        Ok(result)
    }
    pub fn finish(&self, op: &Op, first: Value) -> Result<Value, Failure> {
        let Some(completion) = &op.completion else { return Ok(first) };
        let clock = self.transport.clock();
        let timing = self.transport.timing;
        let until = clock.now() + timing.deadline;
        let mut value = first;
        let mut path = value["path"].as_str().map(str::to_owned);
        let mut polled = false;
        loop {
            if value.is_object()
                && value["path"].is_null()
                && let Some(path) = &path
            {
                value["path"] = json!(path);
            }
            match completion {
                Completion::Operation => {
                    if !value["error"].is_null() {
                        return Err(Failure::remote(format!("operation failed: {}", value["error"]), value));
                    }
                    if value["done"] == true {
                        return Ok(value);
                    }
                }
                Completion::State { field, success, failure } => {
                    if let Some(state) = value[field].as_str() {
                        if success.iter().any(|s| s == state) {
                            return Ok(value);
                        }
                        if failure.iter().any(|s| s == state) {
                            return Err(Failure::remote(format!("task {state}"), value));
                        }
                    }
                }
            }
            if let Some(new) = value["path"].as_str() {
                path = Some(new.into());
            }
            let path = path.as_deref().ok_or_else(|| Failure::unavailable("response has no resource path to poll"))?;
            let (reader, captures) = self.registry.reader_of(op, path).map_err(|_| {
                Failure::unavailable(format!("no reader matches task path {path}")).with("path", json!(path))
            })?;
            let call = Call::new(reader, decode_captures(captures.0), vec![], None)?;
            let pending = |detail| {
                Failure::new(8, "pending", format!("resource is not done after {} s", timing.deadline.as_secs_f64()))
                    .with("detail", detail)
            };
            if clock.now() >= until {
                return Err(pending(value));
            }
            if polled {
                clock.sleep(timing.poll_interval.min(until.saturating_duration_since(clock.now())));
            }
            polled = true;
            if clock.now() >= until {
                continue;
            }
            let call = call.with_deadline(until);
            value = match self.send(&call) {
                Ok(reply) => reply.json()?,
                Err(mut failure) => {
                    if clock.now() >= until {
                        failure.code = 8;
                        failure.error["kind"] = json!("pending");
                    }
                    return Err(failure.with("last", value));
                }
            };
        }
    }
}
pub fn decode_captures(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    pairs
        .into_iter()
        .map(|(k, v)| (k, percent_encoding::percent_decode_str(&v).decode_utf8_lossy().into_owned()))
        .collect()
}
impl Executor<'_> {
    pub fn resume(&self, command: String) {
        self.acceptance.borrow_mut().resume(command);
    }
    pub fn note(&self, message: &str) {
        self.out.note(message);
    }
    pub fn download(&self, location: &str, to: &std::path::Path, overwrite: bool) -> Result<u64, Failure> {
        self.transport.download(location, to, overwrite)
    }
}
fn request_timeout(call: &Call<'_>) -> Duration {
    Duration::from_secs(if call.has_binary_body() {
        600
    } else if call.op().method == Method::Get {
        30
    } else {
        120
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use oc_registry::Payload;

    #[test]
    fn empty_success_ignores_every_declared_payload_kind() {
        let server = httpmock::MockServer::start();
        let m = server.mock(|_, t| {
            t.status(200).body("");
        });
        let output = Output::new(false);
        let acceptance = std::cell::RefCell::new(Acceptance::default());
        let ex = Executor::new(
            &crate::REGISTRY,
            Some(server.base_url()),
            None,
            Timing { deadline: Duration::ZERO, poll_interval: Duration::ZERO, retry_delay: Some(Duration::ZERO) },
            &output,
            &acceptance,
        )
        .unwrap();
        for response in
            [Payload::Empty, Payload::Json { shape: oc_registry::Shape::Any }, Payload::Text, Payload::Binary]
        {
            let mut op = crate::REGISTRY.op("universes.get-place-universe").unwrap().clone();
            op.response = response;
            let call = Call::new(&op, vec![], vec![("placeId".into(), "42".into())], None).unwrap();
            assert!(ex.send(&call).unwrap().json().unwrap().is_null(), "empty 2xx bodies ignore payload metadata");
        }
        m.assert_calls(4);
    }
    #[test]
    fn download_needs_no_registry_operation_or_key() {
        let server = httpmock::MockServer::start();
        let m = server.mock(|w, t| {
            w.path("/file").header_missing("x-api-key");
            t.body("bytes");
        });
        let output = Output::new(false);
        let acceptance = std::cell::RefCell::new(Acceptance::default());
        let ex = Executor::new(
            &crate::REGISTRY,
            Some(server.base_url()),
            None,
            Timing { deadline: Duration::ZERO, poll_interval: Duration::ZERO, retry_delay: Some(Duration::ZERO) },
            &output,
            &acceptance,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset");
        assert_eq!(ex.download(&format!("{}/file", server.base_url()), &path, false).unwrap(), 5);
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
        m.assert_calls(1);
    }
    #[test]
    fn binary_uploads_have_ten_minutes_and_polls_keep_short_budgets() {
        let publish = crate::REGISTRY.op("universes.create-place-version").unwrap();
        let binary = Call::new(
            publish,
            vec![],
            vec![("universeId".into(), "1".into()), ("placeId".into(), "2".into())],
            Some(crate::call::BodyInput::Binary { content_type: "application/octet-stream".into(), bytes: vec![1] }),
        )
        .unwrap();
        assert_eq!(request_timeout(&binary), Duration::from_secs(600), "binary calls select the longer request budget");
        let read = Call::new(
            crate::REGISTRY.op("assets.get-operation").unwrap(),
            vec![],
            vec![("operationId".into(), "x".into())],
            None,
        )
        .unwrap();
        assert_eq!(request_timeout(&read), Duration::from_secs(30), "polls retain short budgets");
    }
}
