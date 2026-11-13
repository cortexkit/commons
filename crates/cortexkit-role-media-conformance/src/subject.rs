//! Adapter controls. Hook errors must not be converted into wire refusals.

use async_trait::async_trait;
use cortexkit_role_media::describe::Engine;
use serde_json::Value;

use crate::peers::{RecordingRoutingPeer, RecordingVendor};

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Capabilities {
    pub has_speech_check: bool,
    pub retries_transient: bool,
    pub jobs_survive_restart: bool,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_has_speech_check(mut self, value: bool) -> Self {
        self.has_speech_check = value;
        self
    }
    pub fn with_retries_transient(mut self, value: bool) -> Self {
        self.retries_transient = value;
        self
    }
    pub fn with_jobs_survive_restart(mut self, value: bool) -> Self {
        self.jobs_survive_restart = value;
        self
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Request {
    pub method: String,
    pub params: Value,
}
impl Request {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SubjectError {
    pub message: String,
    /// The unmodified ERROR-frame body, absent for adapter/setup failures.
    pub refusal: Option<Value>,
}
impl SubjectError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            refusal: None,
        }
    }
    pub fn refused(body: Value) -> Self {
        Self {
            message: "wire refusal".into(),
            refusal: Some(body),
        }
    }
    pub fn unsupported(hook: &str) -> Self {
        Self::new(format!("subject hook {hook} is not implemented"))
    }
}
impl std::fmt::Display for SubjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for SubjectError {}
pub type SubjectResult<T> = Result<T, SubjectError>;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Agent {
    pub agent_id: String,
    pub owner: String,
    pub context_readers: Vec<String>,
}
impl Agent {
    pub fn new(agent_id: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            owner: owner.into(),
            context_readers: Vec::new(),
        }
    }
    pub fn with_context_readers(mut self, readers: Vec<String>) -> Self {
        self.context_readers = readers;
        self
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Principals {
    pub usage_reader: String,
    pub agents: Vec<Agent>,
}
impl Principals {
    pub fn new(usage_reader: impl Into<String>, agents: Vec<Agent>) -> Self {
        Self {
            usage_reader: usage_reader.into(),
            agents,
        }
    }
}

/// Ordered engine events. A pause blocks a started attempt, not acceptance.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ScriptStep {
    Output(Value),
    Failure(Value),
    Partial(Value),
    Final(Value),
    Progress(f64),
    Pause,
}

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Script {
    pub steps: Vec<ScriptStep>,
    pub decoded_duration_ms: Option<u64>,
    pub measurements: Option<Vec<Value>>,
    pub vendor_request_id: Option<String>,
    pub speech_check: Option<bool>,
}
impl Script {
    pub fn new(steps: Vec<ScriptStep>) -> Self {
        Self {
            steps,
            ..Self::default()
        }
    }
    pub fn with_decoded_duration_ms(mut self, value: u64) -> Self {
        self.decoded_duration_ms = Some(value);
        self
    }
    pub fn with_measurements(mut self, value: Vec<Value>) -> Self {
        self.measurements = Some(value);
        self
    }
    pub fn with_vendor_request_id(mut self, value: impl Into<String>) -> Self {
        self.vendor_request_id = Some(value.into());
        self
    }
    /// True means speech was detected; false means no speech was detected.
    pub fn with_speech_check(mut self, value: bool) -> Self {
        self.speech_check = Some(value);
        self
    }
}

/// Drained stream events and the terminal transport state observed by an adapter.
/// An accepted stream closed by restart has `closed` true and no answer.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct StreamObservation {
    pub events: Vec<Value>,
    /// The unmodified terminal RESPONSE body, including the `result` envelope.
    /// STREAM_DATA events and referenced output-object contents stay unwrapped.
    pub answer: Option<Value>,
    pub closed: bool,
}
impl StreamObservation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_events(mut self, events: Vec<Value>) -> Self {
        self.events = events;
        self
    }
    pub fn with_answer(mut self, answer: Value) -> Self {
        self.answer = Some(answer);
        self
    }
    pub fn with_closed(mut self, closed: bool) -> Self {
        self.closed = closed;
        self
    }
}

#[async_trait]
pub trait MediaSubject: Send + Sync {
    /// Set the agent head's fork readiness: None is ready; fork_unsupported means
    /// its runner lacks fork answers, and fork_refused means fork preparation was refused.
    async fn configure_voice_front(
        &self,
        _agent_id: &str,
        _unavailable_reason: Option<&str>,
    ) -> SubjectResult<()> {
        Err(SubjectError::unsupported("configure_voice_front"))
    }
    /// Subscribe to real realtime.watch pushes for an already recorded call.
    async fn open_watch(&self, _principal: &str, _call_id: &str) -> SubjectResult<()> {
        Err(SubjectError::unsupported("open_watch"))
    }
    /// Drain watch pushes without blocking; opening a watch need not replay earlier events.
    async fn poll_watch(&self, _principal: &str, _call_id: &str) -> SubjectResult<Vec<Value>> {
        Err(SubjectError::unsupported("poll_watch"))
    }
    /// Open media.stream_transcribe; return after acceptance or an opening ERROR.
    /// Events and the terminal RESPONSE remain available through poll_stream.
    async fn open_stream(&self, _principal: &str, _params: Value) -> SubjectResult<()> {
        Err(SubjectError::unsupported("open_stream"))
    }
    #[deprecated(since = "0.2.5", note = "Send media.stream_audio through call_as")]
    async fn send_stream_frame(
        &self,
        _principal: &str,
        _stream_id: &str,
        _frame: Value,
    ) -> SubjectResult<()> {
        Err(SubjectError::unsupported("send_stream_frame"))
    }
    /// Decoded audio chunks actually forwarded to the engine, in consumption order.
    /// Chunk boundaries may differ from request boundaries.
    /// Observe the engine boundary; do not sort requests or include refused frames.
    async fn stream_audio_inputs(
        &self,
        _principal: &str,
        _stream_id: &str,
    ) -> SubjectResult<Vec<Vec<u8>>> {
        Err(SubjectError::unsupported("stream_audio_inputs"))
    }
    /// Drain events without blocking. The terminal answer, when present, remains readable.
    async fn poll_stream(
        &self,
        _principal: &str,
        _stream_id: &str,
    ) -> SubjectResult<StreamObservation> {
        Err(SubjectError::unsupported("poll_stream"))
    }
    async fn close_stream(&self, _principal: &str, _stream_id: &str) -> SubjectResult<()> {
        Err(SubjectError::unsupported("close_stream"))
    }
    /// Inject one pre-acceptance lane refusal, without imposing a lane capacity.
    /// Only realtime_lane_full is allowed; other codes return a named hook error.
    async fn refuse_next_stream(&self, _code: &str) -> SubjectResult<()> {
        Err(SubjectError::unsupported("refuse_next_stream"))
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
    }
    async fn configure_engines(&self, _engines: Vec<Engine>) -> SubjectResult<()> {
        Err(SubjectError::unsupported("configure_engines"))
    }
    async fn engine_declaration(&self) -> SubjectResult<Vec<Engine>> {
        Err(SubjectError::unsupported("engine_declaration"))
    }
    async fn configure_peers(
        &self,
        _routing: RecordingRoutingPeer,
        _vendor: RecordingVendor,
    ) -> SubjectResult<()> {
        Err(SubjectError::unsupported("configure_peers"))
    }
    async fn configure_principals(&self, _principals: Principals) -> SubjectResult<()> {
        Err(SubjectError::unsupported("configure_principals"))
    }
    /// Send a real request and return the unmodified RESPONSE body on success.
    /// Keep the `result` envelope; the runner checks it with `wire::read_reply`.
    /// Refusals use `SubjectError::refused` with the complete ERROR body.
    async fn call_as(&self, _principal: &str, _request: Request) -> SubjectResult<Value> {
        Err(SubjectError::unsupported("call_as"))
    }
    async fn configure_pack(
        &self,
        _agent_id: &str,
        _purpose: &str,
        _pack: Value,
    ) -> SubjectResult<()> {
        Err(SubjectError::unsupported("configure_pack"))
    }
    async fn script_engine(&self, _model: &str, _script: Script) -> SubjectResult<()> {
        Err(SubjectError::unsupported("script_engine"))
    }
    async fn engine_inputs(&self, _model: &str) -> SubjectResult<Vec<Value>> {
        Err(SubjectError::unsupported("engine_inputs"))
    }
    async fn hold_engine(&self, _model: &str) -> SubjectResult<()> {
        Err(SubjectError::unsupported("hold_engine"))
    }
    async fn release_engine(&self, _model: &str) -> SubjectResult<()> {
        Err(SubjectError::unsupported("release_engine"))
    }
    async fn restart(&self) -> SubjectResult<()> {
        Err(SubjectError::unsupported("restart"))
    }
    /// Fire every timer due through the new time, including shared peer timers.
    async fn advance_clock(&self, _ms: u64) -> SubjectResult<()> {
        Err(SubjectError::unsupported("advance_clock"))
    }
    async fn seed_usage_entry(&self, _bytes: Vec<u8>) -> SubjectResult<()> {
        Err(SubjectError::unsupported("seed_usage_entry"))
    }
}
