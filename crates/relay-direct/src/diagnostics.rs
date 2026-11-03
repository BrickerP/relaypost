//! Bounded, in-memory network observability. No packet, identity, password or
//! user-file payload is accepted by this recorder and nothing is persisted.
use std::collections::VecDeque;
use std::fmt::{self, Write};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

const EVENT_LIMIT: usize = 32;
const TEXT_LIMIT: usize = 1024;
const SUBSCRIBER_LIMIT: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct DirectDiagnostics {
    pub search_started: u64,
    pub services_found: u64,
    pub services_resolved: u64,
    pub announcements: u64,
    pub probe_started: u64,
    pub probe_succeeded: u64,
    pub probe_failed: u64,
    pub probe_discarded: u64,
    pub mdns_io_errors: u64,
    pub valid_candidates: usize,
    pub active_probes: usize,
    pub active_jobs: usize,
    pub healthy_peers: usize,
    pub mdns_log_capture: bool,
    pub last_probe_failure: Option<String>,
    pub last_mdns_io_error: Option<String>,
    pub events: Vec<NetworkDiagnosticEvent>,
}

#[derive(Clone, Debug)]
pub struct NetworkDiagnosticEvent {
    pub elapsed_ms: u64,
    pub stage: String,
    pub message: String,
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    SearchStarted, Found, Resolved, Announced, SearchStopped, Removed,
    ProbeStarted, ProbeSucceeded, ProbeFailed, ProbeDiscarded, MdnsIo,
    DiscoveryError,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::SearchStarted => "search_started", Self::Found => "service_found",
            Self::Resolved => "service_resolved", Self::Announced => "announced",
            Self::SearchStopped => "search_stopped", Self::Removed => "service_removed",
            Self::ProbeStarted => "probe_started", Self::ProbeSucceeded => "probe_succeeded",
            Self::ProbeFailed => "probe_failed", Self::ProbeDiscarded => "probe_discarded",
            Self::MdnsIo => "mdns_io_error", Self::DiscoveryError => "discovery_error",
        }
    }
}

struct Trace {
    totals: DirectDiagnostics,
    events: VecDeque<NetworkDiagnosticEvent>,
    last_notify: Instant,
}

pub(crate) struct Recorder {
    started: Instant,
    trace: Mutex<Trace>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Recorder {
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let now = Instant::now();
        let recorder = Arc::new(Self { started: now, trace: Mutex::new(Trace {
            totals: DirectDiagnostics::default(), events: VecDeque::new(), last_notify: now,
        }), notify });
        let capture = subscribe(&recorder);
        recorder.trace.lock().unwrap_or_else(|e| e.into_inner()).totals.mdns_log_capture = capture;
        recorder
    }

    pub fn note(&self, stage: Stage, message: &str) {
        let message = bounded_text(message);
        let notify = {
            let mut trace = self.trace.lock().unwrap_or_else(|e| e.into_inner());
            let totals = &mut trace.totals;
            let counter = match stage {
                Stage::SearchStarted => Some(&mut totals.search_started),
                Stage::Found => Some(&mut totals.services_found),
                Stage::Resolved => Some(&mut totals.services_resolved),
                Stage::Announced => Some(&mut totals.announcements),
                Stage::ProbeStarted => Some(&mut totals.probe_started),
                Stage::ProbeSucceeded => Some(&mut totals.probe_succeeded),
                Stage::ProbeFailed => Some(&mut totals.probe_failed),
                Stage::ProbeDiscarded => Some(&mut totals.probe_discarded),
                Stage::MdnsIo => Some(&mut totals.mdns_io_errors),
                _ => None,
            };
            if let Some(counter) = counter { *counter = counter.saturating_add(1); }
            if matches!(stage, Stage::ProbeFailed) { totals.last_probe_failure = Some(message.clone()); }
            if matches!(stage, Stage::MdnsIo) { totals.last_mdns_io_error = Some(message.clone()); }
            if trace.events.len() == EVENT_LIMIT { trace.events.pop_front(); }
            trace.events.push_back(NetworkDiagnosticEvent {
                elapsed_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                stage: stage.name().to_owned(), message,
            });
            let notify = trace.last_notify.elapsed() >= Duration::from_millis(250);
            if notify { trace.last_notify = Instant::now(); }
            notify
        };
        if notify { (self.notify)(); }
    }

    pub fn snapshot(&self) -> DirectDiagnostics {
        let trace = self.trace.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = trace.totals.clone();
        result.events = trace.events.iter().cloned().collect();
        result
    }
}

fn bounded_text(value: &str) -> String {
    value.chars().take(TEXT_LIMIT).map(|c| if c.is_control() { ' ' } else { c }).collect()
}

struct BoundedFormat { text: String, count: usize }
impl Write for BoundedFormat {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for c in value.chars() {
            if self.count == TEXT_LIMIT { return Err(fmt::Error); }
            self.text.push(if c.is_control() { ' ' } else { c });
            self.count += 1;
        }
        Ok(())
    }
}

struct MdnsLogger;
static LOGGER: MdnsLogger = MdnsLogger;
static INSTALLED: OnceLock<bool> = OnceLock::new();
static SUBSCRIBERS: OnceLock<Mutex<Vec<Weak<Recorder>>>> = OnceLock::new();

fn subscribe(recorder: &Arc<Recorder>) -> bool {
    let installed = *INSTALLED.get_or_init(|| {
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
            true
        } else { false }
    });
    if !installed { return false; }
    let mut subscribers = SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap_or_else(|e| e.into_inner());
    subscribers.retain(|value| value.strong_count() > 0);
    if subscribers.len() >= SUBSCRIBER_LIMIT { return false; }
    subscribers.push(Arc::downgrade(recorder));
    true
}

fn mdns_target(target: &str) -> bool { target == "mdns_sd" || target.starts_with("mdns_sd::") }

fn io_message(message: &str) -> bool {
    // Only library format strings known to contain socket/interface/OS errors.
    // Packet dumps, received TXT/hostnames and all non-mDNS targets are ignored.
    ["failed to set up IPv4 socket:", "failed to set up IPv6 socket:",
        "failed to join multicast on ", "failed to set multicast TTL on ",
        "failed to register ipv4 socket:", "failed to register ipv6 socket:",
        "failed to select from sockets:", "listening socket read failed:",
        "handle_read: socket not available for token ",
        "handle_read: no interface found for pktinfo if_index:",
        "send_dns_outgoing: failed to set multicast interface for IPv4 ",
        "send_dns_outgoing: failed to set multicast interface for IPv6 ",
        "Failed to send unicast to ", "Failed to send to "]
        .iter().any(|prefix| message.starts_with(prefix))
}

impl log::Log for MdnsLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool { mdns_target(metadata.target()) }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) { return; }
        let mut output = BoundedFormat { text: String::new(), count: 0 };
        let _ = fmt::write(&mut output, *record.args());
        if !io_message(&output.text) { return; }
        let Some(subscribers) = SUBSCRIBERS.get() else { return; };
        let live: Vec<_> = {
            let mut subscribers = subscribers.lock().unwrap_or_else(|e| e.into_inner());
            let live: Vec<_> = subscribers.iter().filter_map(Weak::upgrade).collect();
            subscribers.retain(|entry| entry.strong_count() > 0);
            live
        };
        for recorder in live { recorder.note(Stage::MdnsIo, &output.text); }
    }
    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;
    #[test]
    fn records_only_bounded_mdns_io_not_payloads_or_other_targets() {
        let recorder = Recorder::new(Arc::new(|| {}));
        let before = recorder.snapshot().mdns_io_errors;
        LOGGER.log(&log::Record::builder().target("relay_core")
            .args(format_args!("failed to set up IPv4 socket: secret-password-or-file-content")).build());
        LOGGER.log(&log::Record::builder().target("mdns_sd::service_daemon")
            .args(format_args!("Received DNS packet TXT secret-password-or-file-content")).build());
        assert_eq!(recorder.snapshot().mdns_io_errors, before);
        LOGGER.log(&log::Record::builder().target("mdns_sd::service_daemon")
            .args(format_args!("Failed to send to 224.0.0.251:5353 via en0: Permission denied (os error 13)")).build());
        let trace = recorder.snapshot();
        assert!(trace.mdns_io_errors > before);
        assert!(trace.last_mdns_io_error.as_deref().unwrap().contains("os error 13"));
        assert!(!format!("{trace:?}").contains("secret-password-or-file-content"));
        for _ in 0..100 { recorder.note(Stage::ProbeDiscarded, &"界".repeat(2000)); }
        let trace = recorder.snapshot();
        assert_eq!(trace.events.len(), EVENT_LIMIT);
        assert!(trace.events.iter().all(|event| event.message.chars().count() <= TEXT_LIMIT));
    }
}
