use crate::discovery::{Discovery, Event};
use crate::wire::{self, Decision, Empty, Kind, PairRequest, PairStatusRequest, ParcelRequest, Reply};
use crate::*;
use anyhow::{bail, ensure, Context, Result};
use relay_core::{DeliveryState, DirectOfferArtifact, Direction};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_PEERS: usize = 64;
const MAX_OFFERS: usize = 8;
const MAX_CONNECTIONS: usize = 4;
const MAX_BUNDLES: usize = 2;
const MAX_JOBS: usize = 4;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn session_id() -> String {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{:x}-{:x}-{:x}", std::process::id(), time, SESSION_COUNTER.fetch_add(1, Ordering::Relaxed))
}
fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> { value.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

#[derive(Clone)]
struct Endpoint { card: relay_core::PeerCard, addresses: Vec<SocketAddrV4>, service: String, seen: Instant, verified: bool }
struct PairEntry { card: relay_core::PeerCard, decision: Decision, error: Option<String>, created: Instant }
struct OfferEntry { artifact: DirectOfferArtifact, decision: Decision, receiving: bool, created: Instant }
struct State {
    snapshot: DirectSnapshot,
    peers: HashMap<PeerId, Endpoint>,
    pairs: HashMap<String, PairEntry>,
    offers: HashMap<String, OfferEntry>,
    jobs: HashSet<String>,
    staged: HashSet<String>,
}
struct Shared {
    profile: Profile,
    state: Mutex<State>,
    stopped: AtomicBool,
    finished: AtomicBool,
    active: Mutex<HashMap<u64, TcpStream>>,
    connections: AtomicUsize,
    bundles: AtomicUsize,
    jobs: AtomicUsize,
    next_socket: AtomicU64,
    address: Mutex<Option<SocketAddrV4>>,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn edit(&self, change: impl FnOnce(&mut State)) {
        {
            let mut state = lock(&self.state);
            change(&mut state);
            state.snapshot.peers = state.peers.iter().map(|(id, peer)| NearbyPeer {
                id: id.clone(), card: peer.card.clone(), locally_verified: peer.verified,
            }).collect();
            state.snapshot.peers.sort_by(|a,b| a.card.name.cmp(&b.card.name).then(a.id.0.cmp(&b.id.0)));
            state.snapshot.incoming_pairings = state.pairs.iter()
                .map(|(id,p)| IncomingPairing { id: PairingId(id.clone()), card: p.card.clone(), state: match &p.error {
                    Some(message) => IncomingPairingState::Failed { message: message.clone() },
                    None => match p.decision { Decision::Waiting => IncomingPairingState::AwaitingDecision,
                        Decision::Accepted | Decision::Stored => IncomingPairingState::Accepted,
                        Decision::Declined => IncomingPairingState::Declined },
                } }).collect();
            state.snapshot.generation = state.snapshot.generation.wrapping_add(1);
        }
        (self.on_change)();
    }
    fn revision(&self) { self.edit(|s| s.snapshot.profile_revision = s.snapshot.profile_revision.wrapping_add(1)); }
    fn outgoing(&self, id: &str, change: OutgoingTransferState) {
        self.edit(|s| { if let Some(row) = s.snapshot.outgoing.iter_mut().find(|r| r.parcel_id == id) { row.state = change; } });
    }
    fn incoming(&self, id: &str, change: IncomingOfferState) {
        self.edit(|s| { if let Some(row) = s.snapshot.incoming.iter_mut().find(|r| r.parcel_id == id) { row.state = change; } });
    }
    fn track(self: &Arc<Self>, stream: &wire::Stream) -> Result<SocketGuard> {
        let socket = stream.socket.try_clone()?;
        let id = self.next_socket.fetch_add(1, Ordering::Relaxed);
        let mut active = lock(&self.active);
        ensure!(!self.stopped.load(Ordering::Acquire), "Local-network transfers stopped");
        active.insert(id, socket);
        Ok(SocketGuard { shared: self.clone(), id })
    }
    fn rpc<T: Serialize>(self: &Arc<Self>, address: SocketAddrV4, kind: Kind, value: &T) -> Result<Reply> {
        ensure!(!self.stopped.load(Ordering::Acquire), "Local-network transfers stopped");
        let mut stream = wire::connect(address)?;
        let _socket = self.track(&stream)?;
        wire::write_json(&mut stream, kind, value)?;
        wire::read_reply(&mut stream)
    }
    fn endpoint(&self, id: &PeerId) -> Result<Endpoint> {
        lock(&self.state).peers.get(id).cloned().context("That computer is no longer nearby")
    }
    fn start_job(self: &Arc<Self>, key: String, action: impl FnOnce(Arc<Self>) + Send + 'static) -> bool {
        if self.stopped.load(Ordering::Acquire) { return false; }
        {
            let mut state = lock(&self.state);
            if state.jobs.contains(&key) || self.jobs.load(Ordering::Acquire) >= MAX_JOBS { return false; }
            state.jobs.insert(key.clone());
            self.jobs.fetch_add(1, Ordering::AcqRel);
        }
        let shared = self.clone();
        thread::spawn(move || {
            let _guard = JobGuard { shared: shared.clone(), key };
            action(shared);
        });
        true
    }
    fn close_sockets(&self) {
        for stream in lock(&self.active).values() { let _ = stream.shutdown(Shutdown::Both); }
    }
}
struct SocketGuard { shared: Arc<Shared>, id: u64 }
impl Drop for SocketGuard { fn drop(&mut self) { lock(&self.shared.active).remove(&self.id); } }
struct JobGuard { shared: Arc<Shared>, key: String }
impl Drop for JobGuard {
    fn drop(&mut self) {
        lock(&self.shared.state).jobs.remove(&self.key);
        self.shared.jobs.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CountGuard<'a>(&'a AtomicUsize);
impl Drop for CountGuard<'_> { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); } }

pub struct DirectHandle {
    shared: Arc<Shared>,
    commands: mpsc::SyncSender<DirectCommand>,
    done: Mutex<mpsc::Receiver<()>>,
}

impl DirectHandle {
    pub fn spawn_with_config(profile: Profile, on_change: Arc<dyn Fn() + Send + Sync>, config: DirectConfig) -> Self {
        let snapshot = DirectSnapshot { generation: 0, profile_revision: 0,
            service: ServiceState::DormantNoIdentity, visible: true,
            peers: Vec::new(), incoming_pairings: Vec::new(), pairing_attempts: Vec::new(),
            incoming: Vec::new(), outgoing: Vec::new() };
        let shared = Arc::new(Shared { profile,
            state: Mutex::new(State { snapshot, peers: HashMap::new(), pairs: HashMap::new(), offers: HashMap::new(), jobs: HashSet::new(), staged: HashSet::new() }),
            stopped: AtomicBool::new(false), finished: AtomicBool::new(false), active: Mutex::new(HashMap::new()),
            connections: AtomicUsize::new(0), bundles: AtomicUsize::new(0), jobs: AtomicUsize::new(0),
            next_socket: AtomicU64::new(1), address: Mutex::new(None), on_change });
        let (commands, receiver) = mpsc::sync_channel(32);
        let (done_tx, done) = mpsc::sync_channel(1);
        let worker = shared.clone();
        thread::spawn(move || { run(worker.clone(), receiver, config); worker.finished.store(true, Ordering::Release); let _ = done_tx.send(()); });
        Self { shared, commands, done: Mutex::new(done) }
    }
    pub fn snapshot(&self) -> DirectSnapshot { lock(&self.shared.state).snapshot.clone() }
    pub fn command(&self, command: DirectCommand) -> std::result::Result<(), DirectCommandError> {
        if self.shared.stopped.load(Ordering::Acquire) { return Err(DirectCommandError::Stopped); }
        self.commands.try_send(command).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => DirectCommandError::QueueFull,
            mpsc::TrySendError::Disconnected(_) => DirectCommandError::Stopped,
        })
    }
    pub fn shutdown(&self) -> std::result::Result<(), DirectCommandError> {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.close_sockets();
        if self.shared.finished.load(Ordering::Acquire) { return Ok(()); }
        lock(&self.done).recv_timeout(Duration::from_secs(2)).map_err(|_| DirectCommandError::ShutdownTimeout)
    }
}
impl Drop for DirectHandle {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.close_sockets();
    }
}

fn run(shared: Arc<Shared>, commands: mpsc::Receiver<DirectCommand>, config: DirectConfig) {
    let mut listener: Option<TcpListener> = None;
    let mut discovery: Option<Discovery> = None;
    let mut last_refresh = Instant::now() - Duration::from_secs(30);
    let mut last_receipts = Instant::now();
    let mut restart = true;
    while !shared.stopped.load(Ordering::Acquire) {
        if restart {
            restart = false;
            if let Some(mut old) = discovery.take() { old.stop(); }
            listener.take();
            lock(&shared.address).take();
            if shared.profile.public_card().is_ok() {
                shared.edit(|s| s.snapshot.service = ServiceState::Starting);
                match start_listener(&config) {
                    Ok((socket, address)) => {
                        *lock(&shared.address) = Some(address);
                        listener = Some(socket);
                        shared.edit(|s| s.snapshot.service = ServiceState::Ready);
                        if config.discovery {
                            let visible = lock(&shared.state).snapshot.visible;
                            match Discovery::start(address.port(), &session_id(), visible) {
                                Ok(value) => discovery = Some(value),
                                Err(error) => shared.edit(|s| s.snapshot.service = ServiceState::Unavailable {
                                    kind: UnavailableKind::Discovery, message: format!("Nearby discovery is unavailable: {error}") }),
                            }
                        }
                        restore_profile(&shared);
                    }
                    Err(error) => shared.edit(|s| s.snapshot.service = ServiceState::Unavailable {
                        kind: UnavailableKind::Listener, message: format!("Cannot listen for nearby transfers: {error}") }),
                }
            } else { shared.edit(|s| s.snapshot.service = ServiceState::DormantNoIdentity); }
        }
        for command in commands.try_iter().take(16) {
            match command {
                DirectCommand::SetVisible(visible) => {
                    shared.edit(|s| s.snapshot.visible = visible);
                    if let Some(discovery) = discovery.as_mut() {
                        if let Err(error) = discovery.visible(visible) { discovery_error(&shared, error.to_string()); }
                    }
                }
                DirectCommand::RefreshDiscovery => {
                    if let Some(discovery) = discovery.as_mut() {
                        if let Err(error) = discovery.refresh_events() { discovery_error(&shared, error.to_string()); }
                    } else { restart = true; }
                }
                DirectCommand::ReloadProfile => {
                    if listener.is_none() { restart = true; }
                    restore_profile(&shared);
                }
                DirectCommand::RequestPairing { peer } => request_pairing(&shared, peer),
                DirectCommand::AcceptPairing { request } => accept_pairing(&shared, request),
                DirectCommand::DeclinePairing { request } => shared.edit(|s| {
                    if let Some(pair) = s.pairs.get_mut(&request.0) { pair.decision = Decision::Declined; }
                }),
                DirectCommand::Send { peer, parcel_id } => start_send(&shared, peer, parcel_id),
                DirectCommand::AcceptOffer { parcel_id } => shared.edit(|s| {
                    let accepted = if let Some(offer) = s.offers.get_mut(&parcel_id) {
                        if offer.decision == Decision::Waiting && offer.created.elapsed() < Duration::from_secs(600) {
                            offer.decision = Decision::Accepted; true
                        } else { false }
                    } else { false };
                    if !accepted {
                        if let Some(row) = s.snapshot.incoming.iter_mut().find(|r| r.parcel_id == parcel_id) {
                            if matches!(row.state, IncomingOfferState::AwaitingDecision | IncomingOfferState::Failed { .. }) {
                                row.state = IncomingOfferState::Failed { message: "This request is no longer available. Ask the sender to try again.".to_owned(), retryable: false };
                            }
                        }
                    }
                }),
                DirectCommand::DeclineOffer { parcel_id } => shared.edit(|s| {
                    if let Some(offer) = s.offers.get_mut(&parcel_id) {
                        if !offer.receiving && matches!(offer.decision, Decision::Waiting | Decision::Accepted) { offer.decision = Decision::Declined; }
                    }
                    if let Some(row) = s.snapshot.incoming.iter_mut().find(|r| r.parcel_id == parcel_id) {
                        if matches!(row.state, IncomingOfferState::AwaitingDecision | IncomingOfferState::Failed {..}) { row.state = IncomingOfferState::Declined; }
                    }
                }),
                DirectCommand::Retry { parcel_id } => {
                    let peer = lock(&shared.state).snapshot.outgoing.iter().find(|r| r.parcel_id == parcel_id).map(|r| r.peer.clone());
                    if let Some(peer) = peer { start_send(&shared, peer, parcel_id); }
                }
            }
        }
        if let Some(socket) = listener.as_ref() {
            for _ in 0..8 {
                match socket.accept() {
                    Ok((stream, address)) => accept_connection(&shared, stream, address),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        if let Some(discovery) = discovery.as_ref() {
            for event in discovery.drain() {
                match event {
                    Event::Resolved { service, addresses } => probe(&shared, service, addresses),
                    Event::Removed(service) => shared.edit(|s| s.peers.retain(|_, p| p.service != service)),
                    Event::Error(error) => discovery_error(&shared, error),
                }
            }
        }
        if last_refresh.elapsed() >= Duration::from_secs(10) {
            last_refresh = Instant::now();
            let endpoints: Vec<_> = lock(&shared.state).peers.values().cloned().collect();
            for peer in endpoints { probe(&shared, peer.service, peer.addresses); }
            shared.edit(|s| {
                s.peers.retain(|_, peer| peer.seen.elapsed() < Duration::from_secs(35));
                for pair in s.pairs.values_mut() {
                    if pair.decision == Decision::Waiting && pair.created.elapsed() >= Duration::from_secs(600) {
                        pair.decision = Decision::Declined;
                        pair.error = Some("The pairing request expired. Ask the other computer to try again.".to_owned());
                    }
                }
                s.pairs.retain(|_, pair| pair.created.elapsed() < Duration::from_secs(660));
                let expired: HashSet<_> = s.offers.iter().filter(|(_, offer)| !offer.receiving && offer.decision != Decision::Stored && offer.created.elapsed() >= Duration::from_secs(600)).map(|(id, _)| id.clone()).collect();
                s.offers.retain(|id, _| !expired.contains(id));
                for incoming in &mut s.snapshot.incoming {
                    if expired.contains(&incoming.parcel_id) {
                        incoming.state = IncomingOfferState::Failed { message: "This request expired. Ask the sender to try again.".to_owned(), retryable: false };
                    }
                }
            });
        }
        if last_receipts.elapsed() >= Duration::from_secs(2) {
            last_receipts = Instant::now();
            poll_receipts(&shared);
        }
        thread::sleep(Duration::from_millis(50));
    }
    shared.close_sockets();
    if let Some(mut discovery) = discovery { discovery.stop(); }
    listener.take();
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until && (shared.jobs.load(Ordering::Acquire) > 0 || shared.connections.load(Ordering::Acquire) > 0) {
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_listener(config: &DirectConfig) -> Result<(TcpListener, SocketAddrV4)> {
    ensure!(config.listen_addr.ip().is_unspecified() || wire::is_local(*config.listen_addr.ip()), "Listener address must be local");
    let listener = TcpListener::bind(config.listen_addr)?;
    listener.set_nonblocking(true)?;
    match listener.local_addr()? { SocketAddr::V4(addr) => Ok((listener, addr)), _ => bail!("IPv4 listener required") }
}
fn discovery_error(shared: &Shared, message: String) {
    shared.edit(|s| s.snapshot.service = ServiceState::Unavailable { kind: UnavailableKind::Discovery, message });
}

fn restore_profile(shared: &Arc<Shared>) {
    let contacts = shared.profile.contacts().unwrap_or_default();
    let records = shared.profile.deliveries().unwrap_or_default();
    for record in &records {
        if record.direction == Direction::Incoming && record.state == DeliveryState::SavedReceiptReady {
            let _ = shared.profile.finalize_direct_offer(&record.parcel_id);
        }
    }
    let staged = shared.profile.direct_offers().unwrap_or_default();
    shared.edit(|state| {
        state.staged = staged.iter().map(|p| p.inspection.parcel_id.clone()).collect();
        for peer in state.peers.values_mut() {
            peer.verified = contacts.iter().any(|c| c.verified && c.card == peer.card);
        }
        for record in &records {
            if record.direction == Direction::Outgoing {
                if let Some(row) = state.snapshot.outgoing.iter_mut().find(|row| row.parcel_id == record.parcel_id) {
                    if record.state == DeliveryState::Confirmed { row.state = OutgoingTransferState::Confirmed; }
                } else if record.state == DeliveryState::WaitingReceipt {
                    state.snapshot.outgoing.push(OutgoingTransfer { parcel_id: record.parcel_id.clone(),
                        recipient: record.recipient.clone(), peer: PeerId(record.recipient.clone()), state: OutgoingTransferState::WaitingForSave });
                }
            }
            if record.direction == Direction::Incoming && record.state == DeliveryState::SavedReceiptReady {
                state.offers.remove(&record.parcel_id);
            }
        }
        for parcel in staged.into_iter().take(MAX_OFFERS) {
            let id = &parcel.inspection.parcel_id;
            let name = contacts.iter().find(|c| c.card.fingerprint == parcel.inspection.sender)
                .map(|c| c.card.name.clone()).unwrap_or_else(|| "Verified sender".to_owned());
            if let Some(row) = state.snapshot.incoming.iter_mut().find(|row| &row.parcel_id == id) {
                row.state = IncomingOfferState::ReadyToSave { parcel_path: parcel.parcel_path, inspection: parcel.inspection };
            } else {
                state.snapshot.incoming.push(IncomingOffer { parcel_id: id.clone(), sender_name: name,
                    sender_fingerprint: parcel.inspection.sender.clone(), files: parcel.offer.files, total_bytes: parcel.offer.total_bytes,
                    state: IncomingOfferState::ReadyToSave { parcel_path: parcel.parcel_path, inspection: parcel.inspection } });
            }
        }
        // Snapshots are observations, not a second persistent delivery index.
        if state.snapshot.incoming.len() > 64 {
            let excess = state.snapshot.incoming.len() - 64;
            state.snapshot.incoming.drain(..excess);
        }
    });
}

fn probe(shared: &Arc<Shared>, service: String, addresses: Vec<SocketAddrV4>) {
    let key = format!("probe:{service}");
    shared.start_job(key, move |shared| {
        let own = match shared.profile.public_card() { Ok(card) => card, Err(_) => return };
        for address in &addresses {
            if shared.stopped.load(Ordering::Acquire) { return; }
            let Ok(Reply::Card { card }) = shared.rpc(*address, Kind::Card, &Empty {}) else { continue; };
            if card.fingerprint == own.fingerprint { continue; }
            let Ok(contact) = shared.profile.inspect_contact_card(&card) else { continue; };
            let id = PeerId(card.fingerprint.clone());
            let mut ordered = vec![*address];
            ordered.extend(addresses.iter().filter(|a| *a != address).copied());
            shared.edit(|state| {
                if state.peers.len() < MAX_PEERS || state.peers.contains_key(&id) {
                    state.snapshot.service = ServiceState::Ready;
                    state.peers.insert(id.clone(), Endpoint { card: contact.card, verified: contact.verified,
                        addresses: ordered, service: service.clone(), seen: Instant::now() });
                }
            });
            return;
        }
    });
}

fn accept_connection(shared: &Arc<Shared>, stream: TcpStream, address: SocketAddr) {
    let SocketAddr::V4(address) = address else { return; };
    if !wire::is_local(*address.ip()) || shared.connections.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
        if wire::is_local(*address.ip()) { shared.connections.fetch_sub(1, Ordering::AcqRel); }
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }
    let shared = shared.clone();
    thread::spawn(move || {
        let _count = CountGuard(&shared.connections);
        let Ok(mut stream) = wire::Stream::new(stream) else { return; };
        let Ok(_socket) = shared.track(&stream) else { return; };
        match handle_request(&shared, &mut stream) {
            Ok(reply) => { let _ = wire::write_json(&mut stream, Kind::Reply, &reply); }
            Err(_) => {
                // Do not expose profile paths or detailed local validation errors
                // to a network caller. Local UI state retains actionable errors.
                let _ = wire::write_json(&mut stream, Kind::Reply, &Reply::Rejected {
                    message: "Request rejected. Check device verification, consent and the transfer package.".to_owned(),
                });
            }
        }
    });
}

fn handle_request(shared: &Arc<Shared>, stream: &mut wire::Stream) -> Result<Reply> {
    let (kind, length) = wire::read_header(stream)?;
    match kind {
        Kind::Card => {
            let _: Empty = wire::read_json(stream, length)?;
            ensure!(lock(&shared.state).snapshot.visible, "This computer is not discoverable");
            Ok(Reply::Card { card: shared.profile.public_card()? })
        }
        Kind::Pair => {
            let request: PairRequest = wire::read_json(stream, length)?;
            shared.profile.inspect_contact_card(&request.card)?;
            let mut result = None;
            shared.edit(|state| {
                if !state.snapshot.visible { return; }
                if let Some((id, pair)) = state.pairs.iter().find(|(_, pair)| pair.card == request.card && pair.decision != Decision::Declined) {
                    result = Some((id.clone(), pair.decision)); return;
                }
                state.pairs.retain(|_, pair| pair.created.elapsed() < Duration::from_secs(660));
                if state.pairs.values().filter(|pair| pair.decision == Decision::Waiting).count() >= MAX_OFFERS { return; }
                if state.pairs.len() >= MAX_PEERS {
                    let oldest = state.pairs.iter().filter(|(_, pair)| pair.decision != Decision::Waiting)
                        .min_by_key(|(_, pair)| pair.created).map(|(id, _)| id.clone());
                    if let Some(oldest) = oldest { state.pairs.remove(&oldest); } else { return; }
                }
                let id = session_id();
                state.pairs.insert(id.clone(), PairEntry { card: request.card, decision: Decision::Waiting, error: None, created: Instant::now() });
                result = Some((id, Decision::Waiting));
            });
            let (request_id, decision) = result.context("No pairing request can be accepted now")?;
            Ok(Reply::Pairing { request_id, decision })
        }
        Kind::PairStatus => {
            let request: PairStatusRequest = wire::read_json(stream, length)?;
            ensure!(request.request_id.len() <= 128, "Invalid pairing ID");
            let state = lock(&shared.state);
            let pair = state.pairs.get(&request.request_id).context("Pairing request expired")?;
            Ok(Reply::Pairing { request_id: request.request_id, decision: pair.decision })
        }
        Kind::Offer => {
            let artifact = wire::read_offer(stream, length)?;
            let inspected = shared.profile.inspect_direct_offer(&artifact)?;
            let own = shared.profile.public_card()?;
            ensure!(inspected.recipient == own.fingerprint, "Package is addressed to another computer");
            if let Some(staged) = shared.profile.existing_direct_offer(&artifact)? {
                shared.incoming(&inspected.parcel_id, IncomingOfferState::ReadyToSave {
                    parcel_path: staged.parcel_path, inspection: staged.inspection,
                });
                return Ok(Reply::Offer { parcel_id: inspected.parcel_id, decision: Decision::Stored });
            }
            let contacts = shared.profile.contacts()?;
            let sender_name = contacts.iter().find(|c| c.card.fingerprint == inspected.sender)
                .map(|c| c.card.name.clone()).context("Sender not present")?;
            let mut result = None;
            shared.edit(|state| {
                if !state.snapshot.visible { return; }
                if let Some(existing) = state.offers.get(&inspected.parcel_id) {
                    if existing.artifact.envelope_json != artifact.envelope_json ||
                        existing.artifact.envelope_minisig != artifact.envelope_minisig ||
                        existing.artifact.bundle_bytes != artifact.bundle_bytes { return; }
                    if existing.decision != Decision::Declined {
                        let decision = existing.decision;
                        if decision == Decision::Waiting {
                            if let Some(row) = state.snapshot.incoming.iter_mut().find(|row| row.parcel_id == inspected.parcel_id) {
                                if matches!(row.state, IncomingOfferState::Failed { .. }) { row.state = IncomingOfferState::AwaitingDecision; }
                            }
                        }
                        result = Some(decision); return;
                    }
                }
                state.offers.retain(|_, value| value.decision != Decision::Declined);
                let pending: HashSet<_> = state.offers.keys().chain(state.staged.iter()).collect();
                if pending.len() >= MAX_OFFERS { return; }
                state.offers.insert(inspected.parcel_id.clone(), OfferEntry { artifact,
                    decision: Decision::Waiting, receiving: false, created: Instant::now() });
                let row = IncomingOffer { parcel_id: inspected.parcel_id.clone(), sender_name,
                    sender_fingerprint: inspected.sender.clone(), files: inspected.files,
                    total_bytes: inspected.total_bytes, state: IncomingOfferState::AwaitingDecision };
                if let Some(old) = state.snapshot.incoming.iter_mut().find(|r| r.parcel_id == inspected.parcel_id) { *old = row; }
                else { state.snapshot.incoming.push(row); }
                result = Some(Decision::Waiting);
            });
            Ok(Reply::Offer { parcel_id: inspected.parcel_id, decision: result.context("No incoming request can be accepted now")? })
        }
        Kind::OfferStatus => {
            let request: ParcelRequest = wire::read_json(stream, length)?;
            ensure!(wire::valid_id(&request.parcel_id), "Invalid parcel ID");
            let decision = lock(&shared.state).offers.get(&request.parcel_id).map(|o| o.decision)
                .context("Transfer request expired")?;
            Ok(Reply::Offer { parcel_id: request.parcel_id, decision })
        }
        Kind::Bundle => receive_bundle(shared, stream, length),
        Kind::Receipt => {
            let request: ParcelRequest = wire::read_json(stream, length)?;
            ensure!(wire::valid_id(&request.parcel_id), "Invalid parcel ID");
            let artifact = shared.profile.receipt_artifact(&request.parcel_id)?;
            Ok(Reply::Receipt { artifact })
        }
        Kind::Reply => bail!("Replies are not requests"),
    }
}

fn receive_bundle(shared: &Arc<Shared>, stream: &mut wire::Stream, length: u64) -> Result<Reply> {
    ensure!(length >= 64, "Package frame is truncated");
    let mut id_bytes = [0; 64];
    stream.read_exact(&mut id_bytes)?;
    let id = std::str::from_utf8(&id_bytes)?.to_owned();
    ensure!(wire::valid_id(&id), "Invalid parcel ID");
    let previous_bundles = shared.bundles.fetch_add(1, Ordering::AcqRel);
    let _bundle_count = CountGuard(&shared.bundles);
    ensure!(previous_bundles < MAX_BUNDLES, "Too many concurrent packages");
    let artifact = {
        let mut state = lock(&shared.state);
        let offer = state.offers.get_mut(&id).context("An accepted offer is required")?;
        ensure!(offer.decision == Decision::Accepted && !offer.receiving, "Receiver has not accepted this package");
        ensure!(offer.artifact.bundle_bytes == length - 64, "Package size does not match its offer");
        offer.receiving = true;
        offer.artifact.clone()
    };
    let result = (|| -> Result<_> {
        let mut temporary = tempfile::NamedTempFile::new()?;
        shared.incoming(&id, IncomingOfferState::Receiving { received: 0, total: artifact.bundle_bytes });
        let mut last = Instant::now();
        wire::copy_exact(stream, temporary.as_file_mut(), artifact.bundle_bytes, &shared.stopped, |received| {
            if last.elapsed() >= Duration::from_millis(100) || received == artifact.bundle_bytes {
                last = Instant::now(); shared.incoming(&id, IncomingOfferState::Receiving { received, total: artifact.bundle_bytes });
            }
        })?;
        temporary.as_file().sync_all()?;
        let staged = shared.profile.stage_direct_bundle(&artifact, temporary.path())?;
        shared.incoming(&id, IncomingOfferState::ReadyToSave { parcel_path: staged.parcel_path, inspection: staged.inspection });
        shared.edit(|state| {
            state.staged.insert(id.clone());
            state.snapshot.profile_revision = state.snapshot.profile_revision.wrapping_add(1);
        });
        Ok(Reply::Stored { parcel_id: id.clone() })
    })();
    shared.edit(|state| { if let Some(offer) = state.offers.get_mut(&id) {
        offer.receiving = false;
        offer.decision = if result.is_ok() { Decision::Stored } else { Decision::Waiting };
    }});
    if let Err(error) = &result {
        shared.incoming(&id, IncomingOfferState::Failed { message: error.to_string(), retryable: true });
    }
    result
}

fn pairing_state(shared: &Shared, peer: &PeerId, state: PairingAttemptState) {
    shared.edit(|s| { if let Some(attempt) = s.snapshot.pairing_attempts.iter_mut().find(|r| &r.peer == peer) { attempt.state = state; } });
}
fn request_pairing(shared: &Arc<Shared>, peer: PeerId) {
    if lock(&shared.state).jobs.contains(&format!("pair:{}", peer.0)) { return; }
    let Ok(endpoint) = shared.endpoint(&peer) else { return; };
    shared.edit(|s| {
        let row = PairingAttempt { peer: peer.clone(), card: endpoint.card.clone(), state: PairingAttemptState::Sending };
        if let Some(old) = s.snapshot.pairing_attempts.iter_mut().find(|a| a.peer == peer) { *old = row; }
        else if s.snapshot.pairing_attempts.len() < MAX_PEERS { s.snapshot.pairing_attempts.push(row); }
    });
    let failed_peer = peer.clone();
    let started = shared.start_job(format!("pair:{}", peer.0), move |shared| {
        let result = (|| -> Result<()> {
            shared.profile.import_contact_card(&endpoint.card)?;
            shared.revision();
            let own = shared.profile.public_card()?;
            let address = *endpoint.addresses.first().context("No local address found")?;
            let reply = shared.rpc(address, Kind::Pair, &PairRequest { card: own })?;
            let Reply::Pairing { request_id, mut decision } = reply else { bail!("Unexpected pairing reply"); };
            ensure!(request_id.len() <= 128, "Invalid pairing reply");
            pairing_state(&shared, &peer, PairingAttemptState::WaitingForPeer);
            let until = Instant::now() + Duration::from_secs(600);
            while decision == Decision::Waiting {
                ensure!(!shared.stopped.load(Ordering::Acquire), "Pairing stopped");
                ensure!(Instant::now() < until, "Pairing request expired. Try again with the other computer open.");
                thread::sleep(Duration::from_millis(400));
                let reply = shared.rpc(address, Kind::PairStatus, &PairStatusRequest { request_id: request_id.clone() })?;
                let Reply::Pairing { request_id: returned, decision: value } = reply else { bail!("Unexpected pairing reply"); };
                ensure!(returned == request_id, "Pairing response does not match the request");
                decision = value;
            }
            pairing_state(&shared, &peer, if decision == Decision::Accepted { PairingAttemptState::PeerAccepted } else { PairingAttemptState::Declined });
            Ok(())
        })();
        if let Err(error) = result { pairing_state(&shared, &peer, PairingAttemptState::Failed { message: error.to_string(), retryable: true }); }
    });
    if !started { pairing_state(shared, &failed_peer, PairingAttemptState::Failed { message: "Nearby transfers are busy. Try adding this computer again shortly.".to_owned(), retryable: true }); }
}

fn accept_pairing(shared: &Arc<Shared>, request: PairingId) {
    let card = lock(&shared.state).pairs.get(&request.0).filter(|p| p.decision == Decision::Waiting && p.created.elapsed() < Duration::from_secs(600)).map(|p| p.card.clone());
    let Some(card) = card else {
        shared.edit(|state| {
            if let Some(pair) = state.pairs.get_mut(&request.0) {
                if pair.decision == Decision::Waiting {
                    pair.decision = Decision::Declined;
                    pair.error = Some("This pairing request expired. Ask the other computer to try again.".to_owned());
                }
            }
        });
        return;
    };
    match shared.profile.import_contact_card(&card) {
        Ok(_) => {
            shared.edit(|state| {
                if let Some(pair) = state.pairs.get_mut(&request.0) { pair.decision = Decision::Accepted; }
                state.snapshot.profile_revision = state.snapshot.profile_revision.wrapping_add(1);
            });
        }
        Err(error) => {
            shared.edit(|state| {
                if let Some(pair) = state.pairs.get_mut(&request.0) {
                    pair.decision = Decision::Declined;
                    pair.error = Some(format!("Cannot add that identity: {error}"));
                }
            });
        }
    }
}

fn start_send(shared: &Arc<Shared>, peer: PeerId, parcel_id: String) {
    if !wire::valid_id(&parcel_id) || lock(&shared.state).jobs.contains(&format!("send:{parcel_id}")) { return; }
    let recipient = peer.0.clone();
    shared.edit(|s| {
        let row = OutgoingTransfer { parcel_id: parcel_id.clone(), recipient, peer: peer.clone(), state: OutgoingTransferState::Offering };
        if let Some(old) = s.snapshot.outgoing.iter_mut().find(|r| r.parcel_id == parcel_id) { *old = row; }
        else { s.snapshot.outgoing.push(row); }
    });
    let failed_id = parcel_id.clone();
    let started = shared.start_job(format!("send:{parcel_id}"), move |shared| {
        let result = send_parcel(&shared, &peer, &parcel_id);
        if let Err(error) = result {
            shared.outgoing(&parcel_id, OutgoingTransferState::Failed { message: error.to_string(), retryable: true });
        }
    });
    if !started { shared.outgoing(&failed_id, OutgoingTransferState::Failed { message: "Nearby transfers are busy. Try again shortly.".to_owned(), retryable: true }); }
}

fn send_parcel(shared: &Arc<Shared>, peer: &PeerId, id: &str) -> Result<()> {
    let endpoint = shared.endpoint(peer)?;
    let contact = shared.profile.inspect_contact_card(&endpoint.card)?;
    ensure!(contact.verified, "Check this computer's complete safety code before sending");
    let record = shared.profile.deliveries()?.into_iter().find(|r| r.parcel_id == id && r.direction == Direction::Outgoing)
        .context("Outgoing transfer was not found")?;
    ensure!(record.recipient == endpoint.card.fingerprint, "Selected device is not the intended recipient");
    ensure!(record.state != DeliveryState::Cancelled, "This local transfer was cancelled");
    if record.state == DeliveryState::Confirmed {
        shared.outgoing(id, OutgoingTransferState::Confirmed); return Ok(());
    }
    let address = *endpoint.addresses.first().context("No local address found")?;
    // A receipt may outlive a broken retained parcel or an ACK lost to a
    // disconnect. Ask for it before rebuilding or retransmitting any bytes.
    if try_receipt(shared, address, id)? { return Ok(()); }
    ensure!(!shared.stopped.load(Ordering::Acquire), "Sending stopped");
    let prepared = shared.profile.prepare_direct_handoff(id)?;
    let reply = shared.rpc(address, Kind::Offer, &prepared.offer)?;
    let Reply::Offer { parcel_id, mut decision } = reply else { bail!("Unexpected transfer request reply"); };
    ensure!(parcel_id == id, "Transfer response does not match the prepared package");
    let until = Instant::now() + Duration::from_secs(600);
    if decision == Decision::Waiting { shared.outgoing(id, OutgoingTransferState::WaitingForRecipient); }
    while decision == Decision::Waiting {
        ensure!(!shared.stopped.load(Ordering::Acquire), "Sending stopped");
        ensure!(Instant::now() < until, "The request expired. Ask the recipient to open Receive and try again.");
        thread::sleep(Duration::from_millis(400));
        let reply = shared.rpc(address, Kind::OfferStatus, &ParcelRequest { parcel_id: id.to_owned() })?;
        let Reply::Offer { parcel_id, decision: value } = reply else { bail!("Unexpected transfer status reply"); };
        ensure!(parcel_id == id, "Transfer status does not match the request");
        decision = value;
    }
    if decision == Decision::Declined { shared.outgoing(id, OutgoingTransferState::Declined); return Ok(()); }
    if decision == Decision::Accepted {
        let mut stream = wire::connect(address)?;
        let _socket = shared.track(&stream)?;
        wire::write_header(&mut stream, Kind::Bundle, prepared.bundle_len() + 64)?;
        stream.write_all(id.as_bytes())?;
        let mut bundle = prepared.open_bundle()?;
        shared.outgoing(id, OutgoingTransferState::Sending { sent: 0, total: prepared.bundle_len() });
        let mut last = Instant::now();
        wire::copy_exact(&mut bundle, &mut stream, prepared.bundle_len(), &shared.stopped, |sent| {
            if last.elapsed() >= Duration::from_millis(100) || sent == prepared.bundle_len() {
                last = Instant::now(); shared.outgoing(id, OutgoingTransferState::Sending { sent, total: prepared.bundle_len() });
            }
        })?;
        let reply = wire::read_reply(&mut stream)?;
        ensure!(matches!(reply, Reply::Stored { parcel_id } if parcel_id == id), "Recipient has not confirmed receiving the complete encrypted package");
    } else { ensure!(decision == Decision::Stored, "Receiver did not accept the transfer"); }
    // This is only an encrypted-stage acknowledgment, never a success receipt.
    shared.profile.record_direct_handoff(id)?;
    shared.outgoing(id, OutgoingTransferState::WaitingForSave);
    shared.revision();
    Ok(())
}

fn try_receipt(shared: &Arc<Shared>, address: SocketAddrV4, id: &str) -> Result<bool> {
    let response = shared.rpc(address, Kind::Receipt, &ParcelRequest { parcel_id: id.to_owned() });
    let Ok(Reply::Receipt { artifact }) = response else { return Ok(false); };
    let public_receipt: serde_json::Value = serde_json::from_str(&artifact.receipt_json)?;
    ensure!(public_receipt.get("parcel_id").and_then(|v| v.as_str()) == Some(id), "Returned confirmation is for another transfer");
    let record = shared.profile.import_receipt_artifact(&artifact)?;
    ensure!(record.parcel_id == id && record.state == DeliveryState::Confirmed, "Returned confirmation does not match this transfer");
    shared.outgoing(id, OutgoingTransferState::Confirmed);
    shared.revision();
    Ok(true)
}

fn poll_receipts(shared: &Arc<Shared>) {
    let worker = shared.clone();
    shared.start_job("receipt-scan".to_owned(), move |_| {
        let Ok(records) = worker.profile.deliveries() else { return; };
        for record in records.into_iter().filter(|r| r.direction == Direction::Outgoing && r.state == DeliveryState::WaitingReceipt).take(64) {
            if worker.stopped.load(Ordering::Acquire) { break; }
            let Ok(peer) = worker.endpoint(&PeerId(record.recipient.clone())) else { continue; };
            if !peer.verified { continue; }
            if let Some(address) = peer.addresses.first() { let _ = try_receipt(&worker, *address, &record.parcel_id); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::Ipv4Addr;

    const PASSWORD: &str = "synthetic direct-transfer test password";

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(15);
        while Instant::now() < until {
            if condition() { return; }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(condition(), "Timed out waiting for a local-network state transition");
    }
    fn node(profile: &Profile) -> DirectHandle {
        let handle = DirectHandle::spawn_with_config(profile.clone(), Arc::new(|| {}), DirectConfig {
            listen_addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), discovery: false,
        });
        wait_until(|| lock(&handle.shared.address).is_some());
        handle
    }
    fn address(node: &DirectHandle) -> SocketAddrV4 { lock(&node.shared.address).unwrap() }
    fn discover(from: &DirectHandle, to: &DirectHandle, fingerprint: &str) {
        probe(&from.shared, format!("loopback-{fingerprint}"), vec![address(to)]);
        wait_until(|| from.snapshot().peers.iter().any(|p| p.card.fingerprint == fingerprint));
    }

    #[test]
    fn loopback_pair_accept_stream_explicit_save_and_signed_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let alice = Profile::open(root.path().join("alice")).unwrap();
        let bob = Profile::open(root.path().join("bob")).unwrap();
        let a_card = alice.initialize("Synthetic sender", PASSWORD).unwrap();
        let b_card = bob.initialize("Synthetic receiver", PASSWORD).unwrap();
        let a = node(&alice); let b = node(&bob);
        discover(&a, &b, &b_card.fingerprint); discover(&b, &a, &a_card.fingerprint);
        assert!(alice.contacts().unwrap().is_empty(), "Passive discovery must not import or trust contacts");
        a.command(DirectCommand::RequestPairing { peer: PeerId(b_card.fingerprint.clone()) }).unwrap();
        wait_until(|| !b.snapshot().incoming_pairings.is_empty());
        assert!(bob.contacts().unwrap().is_empty(), "Pairing requires recipient consent");
        let request = b.snapshot().incoming_pairings[0].id.clone();
        b.command(DirectCommand::AcceptPairing { request }).unwrap();
        wait_until(|| a.snapshot().pairing_attempts.iter().any(|p| matches!(p.state, PairingAttemptState::PeerAccepted)));
        assert!(!alice.contacts().unwrap()[0].verified);
        assert!(!bob.contacts().unwrap()[0].verified);
        // Each real user independently compares the entire fingerprint. Neither
        // accepting a card nor finding an endpoint performs this trust action.
        alice.trust_contact(&b_card.fingerprint).unwrap();
        bob.trust_contact(&a_card.fingerprint).unwrap();
        a.command(DirectCommand::ReloadProfile).unwrap();
        b.command(DirectCommand::ReloadProfile).unwrap();
        let source = root.path().join("payload.txt");
        fs::write(&source, b"a real encrypted loopback handoff").unwrap();
        let parcel = alice.seal(&[source.clone()], &b_card.fingerprint, PASSWORD).unwrap();
        a.command(DirectCommand::Send { peer: PeerId(b_card.fingerprint.clone()), parcel_id: parcel.parcel_id.clone() }).unwrap();
        wait_until(|| b.snapshot().incoming.iter().any(|p| p.parcel_id == parcel.parcel_id && matches!(p.state, IncomingOfferState::AwaitingDecision)));
        assert!(bob.direct_offers().unwrap().is_empty());
        assert!(bob.receipt_artifact(&parcel.parcel_id).is_err());
        // A client attempting to bypass Accept cannot upload any large body.
        let prepared = alice.prepare_direct_handoff(&parcel.parcel_id).unwrap();
        for _ in 0..3 {
            let mut premature = wire::connect(address(&b)).unwrap();
            wire::write_header(&mut premature, Kind::Bundle, prepared.bundle_len() + 64).unwrap();
            premature.write_all(parcel.parcel_id.as_bytes()).unwrap();
            assert!(wire::read_reply(&mut premature).is_err());
        }
        assert!(bob.direct_offers().unwrap().is_empty());
        b.command(DirectCommand::AcceptOffer { parcel_id: parcel.parcel_id.clone() }).unwrap();
        wait_until(|| b.snapshot().incoming.iter().any(|p| p.parcel_id == parcel.parcel_id && matches!(p.state, IncomingOfferState::ReadyToSave {..})));
        wait_until(|| alice.deliveries().unwrap().iter().any(|r| r.parcel_id == parcel.parcel_id && r.state == DeliveryState::WaitingReceipt));
        assert!(bob.deliveries().unwrap().is_empty(), "Staging encrypted bytes is not a saved delivery");
        assert!(bob.receipt_artifact(&parcel.parcel_id).is_err());
        let staged = bob.direct_offers().unwrap().pop().unwrap();
        let saved = bob.receive_parcel(&staged.parcel_path, PASSWORD).unwrap();
        assert_eq!(saved.record.state, DeliveryState::SavedReceiptReady);
        assert_eq!(fs::read(source).unwrap(), fs::read(saved.record.received_path.unwrap().join("payload.txt")).unwrap());
        b.command(DirectCommand::ReloadProfile).unwrap();
        wait_until(|| alice.deliveries().unwrap().iter().any(|r| r.parcel_id == parcel.parcel_id && r.state == DeliveryState::Confirmed));
        wait_until(|| a.snapshot().outgoing.iter().any(|p| p.parcel_id == parcel.parcel_id && matches!(p.state, OutgoingTransferState::Confirmed)));
        a.shutdown().unwrap(); b.shutdown().unwrap();
    }

    #[test]
    fn declined_request_has_no_staged_package_or_success_receipt() {
        let root = tempfile::tempdir().unwrap();
        let alice = Profile::open(root.path().join("alice")).unwrap();
        let bob = Profile::open(root.path().join("bob")).unwrap();
        let ac = alice.initialize("Synthetic Alice", PASSWORD).unwrap();
        let bc = bob.initialize("Synthetic Bob", PASSWORD).unwrap();
        alice.import_contact_card(&bc).unwrap(); alice.trust_contact(&bc.fingerprint).unwrap();
        bob.import_contact_card(&ac).unwrap(); bob.trust_contact(&ac.fingerprint).unwrap();
        let a = node(&alice); let b = node(&bob); discover(&a, &b, &bc.fingerprint);
        let source = root.path().join("declined.txt"); fs::write(&source, b"never transferred").unwrap();
        let record = alice.seal(&[source], &bc.fingerprint, PASSWORD).unwrap();
        a.command(DirectCommand::Send { peer: PeerId(bc.fingerprint), parcel_id: record.parcel_id.clone() }).unwrap();
        wait_until(|| !b.snapshot().incoming.is_empty());
        b.command(DirectCommand::DeclineOffer { parcel_id: record.parcel_id.clone() }).unwrap();
        wait_until(|| a.snapshot().outgoing.iter().any(|p| matches!(p.state, OutgoingTransferState::Declined)));
        assert!(bob.direct_offers().unwrap().is_empty());
        assert!(bob.receipt_artifact(&record.parcel_id).is_err());
        assert_eq!(alice.deliveries().unwrap()[0].state, DeliveryState::Ready);
        a.shutdown().unwrap(); b.shutdown().unwrap();
    }

    #[test]
    fn restart_recovers_exact_stage_and_polls_saved_receipt() {
        let root = tempfile::tempdir().unwrap();
        let alice = Profile::open(root.path().join("alice")).unwrap();
        let bob = Profile::open(root.path().join("bob")).unwrap();
        let ac = alice.initialize("Synthetic restart sender", PASSWORD).unwrap();
        let bc = bob.initialize("Synthetic restart receiver", PASSWORD).unwrap();
        alice.import_contact_card(&bc).unwrap(); alice.trust_contact(&bc.fingerprint).unwrap();
        bob.import_contact_card(&ac).unwrap(); bob.trust_contact(&ac.fingerprint).unwrap();
        let source = root.path().join("restart.txt"); fs::write(&source, b"survives application restart").unwrap();
        let record = alice.seal(&[source.clone()], &bc.fingerprint, PASSWORD).unwrap();
        let prepared = alice.prepare_direct_handoff(&record.parcel_id).unwrap();
        let encrypted = root.path().join("encrypted.bundle");
        let mut file = fs::File::create(&encrypted).unwrap();
        std::io::copy(&mut prepared.open_bundle().unwrap(), &mut file).unwrap();
        file.sync_all().unwrap();
        // This is the managed on-disk state after an earlier accepted network
        // transfer. Newly created nodes must restore it without a second save.
        bob.stage_direct_bundle(&prepared.offer, &encrypted).unwrap();
        alice.record_direct_handoff(&record.parcel_id).unwrap();
        let b = node(&bob);
        let a = node(&alice);
        discover(&a, &b, &bc.fingerprint);
        wait_until(|| b.snapshot().incoming.iter().any(|row| row.parcel_id == record.parcel_id && matches!(row.state, IncomingOfferState::ReadyToSave {..})));
        let reply = a.shared.rpc(address(&b), Kind::Offer, &prepared.offer).unwrap();
        assert!(matches!(reply, Reply::Offer { decision: Decision::Stored, .. }));
        assert_eq!(alice.deliveries().unwrap()[0].state, DeliveryState::WaitingReceipt);
        a.shutdown().unwrap();
        let staged = bob.direct_offers().unwrap().pop().unwrap();
        let received = bob.receive_parcel(&staged.parcel_path, PASSWORD).unwrap();
        b.command(DirectCommand::ReloadProfile).unwrap();
        let restarted = node(&Profile::open(root.path().join("alice")).unwrap());
        discover(&restarted, &b, &bc.fingerprint);
        wait_until(|| alice.deliveries().unwrap()[0].state == DeliveryState::Confirmed);
        assert_eq!(fs::read(source).unwrap(), fs::read(received.record.received_path.unwrap().join("restart.txt")).unwrap());
        assert_eq!(bob.deliveries().unwrap().len(), 1);
        restarted.shutdown().unwrap(); b.shutdown().unwrap();
    }

    #[test]
    fn shutdown_interrupts_an_incomplete_network_header() {
        let root = tempfile::tempdir().unwrap();
        let profile = Profile::open(root.path().join("receiver")).unwrap();
        profile.initialize("Synthetic shutdown receiver", PASSWORD).unwrap();
        let receiver = node(&profile);
        let mut blocked = wire::connect(address(&receiver)).unwrap();
        blocked.write_all(b"RLY").unwrap();
        wait_until(|| receiver.shared.connections.load(Ordering::Acquire) > 0);
        let start = Instant::now();
        receiver.shutdown().unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        receiver.shutdown().unwrap();
        let mut byte = [0];
        assert_eq!(blocked.read(&mut byte).unwrap_or(0), 0);
    }


    #[test]
    fn interrupted_bundle_retry_requires_fresh_receiver_consent() {
        let root = tempfile::tempdir().unwrap();
        let alice = Profile::open(root.path().join("alice")).unwrap();
        let bob = Profile::open(root.path().join("bob")).unwrap();
        let ac = alice.initialize("Synthetic retry sender", PASSWORD).unwrap();
        let bc = bob.initialize("Synthetic retry receiver", PASSWORD).unwrap();
        alice.import_contact_card(&bc).unwrap(); alice.trust_contact(&bc.fingerprint).unwrap();
        bob.import_contact_card(&ac).unwrap(); bob.trust_contact(&ac.fingerprint).unwrap();
        let source = root.path().join("retry.txt"); fs::write(&source, b"request consent after interruption").unwrap();
        let record = alice.seal(&[source], &bc.fingerprint, PASSWORD).unwrap();
        let prepared = alice.prepare_direct_handoff(&record.parcel_id).unwrap();
        let a = node(&alice); let b = node(&bob);
        discover(&a, &b, &bc.fingerprint);
        let reply = a.shared.rpc(address(&b), Kind::Offer, &prepared.offer).unwrap();
        assert!(matches!(reply, Reply::Offer { decision: Decision::Waiting, .. }));
        b.command(DirectCommand::AcceptOffer { parcel_id: record.parcel_id.clone() }).unwrap();
        wait_until(|| lock(&b.shared.state).offers.get(&record.parcel_id).is_some_and(|offer| offer.decision == Decision::Accepted));
        let mut interrupted = wire::connect(address(&b)).unwrap();
        wire::write_header(&mut interrupted, Kind::Bundle, prepared.bundle_len() + 64).unwrap();
        interrupted.write_all(record.parcel_id.as_bytes()).unwrap();
        interrupted.write_all(b"x").unwrap();
        interrupted.socket.shutdown(Shutdown::Write).unwrap();
        let _ = wire::read_reply(&mut interrupted);
        wait_until(|| b.snapshot().incoming.iter().any(|row| matches!(row.state, IncomingOfferState::Failed {..})));
        assert!(bob.direct_offers().unwrap().is_empty());
        assert!(bob.receipt_artifact(&record.parcel_id).is_err());
        let retry = a.shared.rpc(address(&b), Kind::Offer, &prepared.offer).unwrap();
        assert!(matches!(retry, Reply::Offer { decision: Decision::Waiting, .. }));
        assert!(b.snapshot().incoming.iter().any(|row| matches!(row.state, IncomingOfferState::AwaitingDecision)));
        // The old Accept does not survive a failed upload.
        let mut denied = wire::connect(address(&b)).unwrap();
        wire::write_header(&mut denied, Kind::Bundle, prepared.bundle_len() + 64).unwrap();
        denied.write_all(record.parcel_id.as_bytes()).unwrap();
        assert!(wire::read_reply(&mut denied).is_err());
        b.command(DirectCommand::AcceptOffer { parcel_id: record.parcel_id.clone() }).unwrap();
        a.command(DirectCommand::Send { peer: PeerId(bc.fingerprint), parcel_id: record.parcel_id.clone() }).unwrap();
        wait_until(|| b.snapshot().incoming.iter().any(|row| matches!(row.state, IncomingOfferState::ReadyToSave {..})));
        assert_eq!(alice.deliveries().unwrap()[0].state, DeliveryState::WaitingReceipt);
        assert!(bob.receipt_artifact(&record.parcel_id).is_err());
        a.shutdown().unwrap(); b.shutdown().unwrap();
    }

}
