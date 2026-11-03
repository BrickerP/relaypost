use crate::components::*;
use crate::theme::*;
use crate::work::{self, Action, Completed, Payload};
use eframe::egui::{self, Color32, RichText, TextEdit, Ui};
use relay_core::{
    Contact, DeliveryState, DemoPhase, DemoSession, DemoSnapshot, Direction, Inspection,
    ParcelRecord, PeerCard, Profile, ReceiveResult, RecoverySummary,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Send,
    Receive,
    Activity,
    People,
    Demo,
}

#[derive(Clone)]
enum Unlock {
    Prepare,
    Receive(PathBuf),
    Recover,
}

struct Pending {
    label: &'static str,
    receiver: Receiver<Result<Completed, String>>,
}

pub struct RelayApp {
    profile: Profile,
    home: PathBuf,
    screen: Screen,
    loaded: bool,
    identity: Option<PeerCard>,
    identity_error: Option<String>,
    identity_missing: bool,
    contacts: Vec<Contact>,
    deliveries: Vec<ParcelRecord>,
    pending: Option<Pending>,
    error: Option<String>,
    notice: String,
    password: Zeroizing<String>,
    confirmation: Zeroizing<String>,
    identity_name: String,
    recipient: String,
    inputs: Vec<PathBuf>,
    selected_bytes: Option<u64>,
    sealed_id: Option<String>,
    sealed_inspection: Option<Inspection>,
    sealed_file_bytes: Option<u64>,
    parcel_input: String,
    inspection: Option<Inspection>,
    received: Option<ReceiveResult>,
    selected_delivery: Option<(String, Direction)>,
    recovery: Option<RecoverySummary>,
    trust_contact: Option<Contact>,
    trust_fingerprint: String,
    cancel_id: Option<String>,
    discard_incomplete: bool,
    unlock: Option<Unlock>,
    after_pairing: Option<Screen>,
    transfer_candidates: Vec<PathBuf>,
    receipt_candidates: Vec<PathBuf>,
    receipt_exported: std::collections::HashSet<String>,
    /// Guided demo session (owned here; moved into worker and returned).
    demo_session: Option<Box<DemoSession>>,
    /// Last known public demo snapshot.
    demo_snapshot: Option<DemoSnapshot>,
}

impl RelayApp {
    pub fn new(context: &eframe::CreationContext<'_>, profile: Profile, home: PathBuf) -> Self {
        crate::theme::configure(&context.egui_ctx);
        let mut app = Self {
            profile,
            home,
            screen: Screen::Home,
            loaded: false,
            identity: None,
            identity_error: None,
            identity_missing: false,
            contacts: Vec::new(),
            deliveries: Vec::new(),
            pending: None,
            error: None,
            notice: String::new(),
            password: Zeroizing::new(String::new()),
            confirmation: Zeroizing::new(String::new()),
            identity_name: String::new(),
            recipient: String::new(),
            inputs: Vec::new(),
            selected_bytes: None,
            sealed_id: None,
            sealed_inspection: None,
            sealed_file_bytes: None,
            parcel_input: String::new(),
            inspection: None,
            received: None,
            selected_delivery: None,
            recovery: None,
            trust_contact: None,
            trust_fingerprint: String::new(),
            cancel_id: None,
            discard_incomplete: false,
            unlock: None,
            after_pairing: None,
            transfer_candidates: Vec::new(),
            receipt_candidates: Vec::new(),
            receipt_exported: std::collections::HashSet::new(),
            demo_session: None,
            demo_snapshot: None,
        };
        app.start(&context.egui_ctx, Action::Refresh);
        app
    }

    fn busy(&self) -> bool {
        self.pending.is_some()
    }

    fn modal_open(&self) -> bool {
        self.unlock.is_some()
            || self.trust_contact.is_some()
            || self.cancel_id.is_some()
            || self.discard_incomplete
            || self.receipt_candidates.len() > 1
    }

    fn start(&mut self, context: &egui::Context, action: Action) {
        if self.busy() {
            return;
        }
        if matches!(&action, Action::Seal(..)) {
            self.invalidate_sealed();
        }
        self.error = None;
        self.notice.clear();
        let label = action.label();
        let profile = self.profile.clone();
        let context = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = work::execute(profile, action).map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.pending = Some(Pending { label, receiver });
    }

    fn poll(&mut self, context: &egui::Context) {
        let result = self.pending.as_ref().and_then(|pending| match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("The background operation stopped unexpectedly. Refresh local records before trying again.".to_owned())),
        });
        let Some(result) = result else { return };
        self.pending = None;
        match result {
            Err(error) => self.error = Some(error),
            Ok(completed) => {
                self.notice = completed.notice;
                if let Some(snapshot) = completed.snapshot {
                    match snapshot.identity {
                        Ok(identity) => {
                            self.identity = Some(identity);
                            self.identity_error = None;
                        }
                        Err(error) => {
                            self.identity = None;
                            self.identity_error = Some(error);
                        }
                    }
                    self.identity_missing = self.identity.is_none()
                        && matches!(
                            std::fs::symlink_metadata(self.home.join("identity")),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                        );
                    self.contacts = snapshot.contacts;
                    self.deliveries = snapshot.deliveries;
                    if let Some(result) = &mut self.received {
                        if let Some(latest) = self.deliveries.iter().find(|record| {
                            record.direction == Direction::Incoming
                                && record.parcel_id == result.record.parcel_id
                        }) {
                            result.record = latest.clone();
                        }
                    }
                    let title = self
                        .identity
                        .as_ref()
                        .map(|card| format!("RelayPost — {}", card.name))
                        .unwrap_or_else(|| "RelayPost".to_owned());
                    context.send_viewport_cmd(egui::ViewportCommand::Title(title));
                    if !self.loaded {
                        self.screen = Screen::Home;
                        self.loaded = true;
                    }
                }
                match completed.payload {
                    Payload::None => {}
                    Payload::Initialized => self.screen = Screen::People,
                    Payload::ContactImported(contact) => {
                        if contact.verified {
                            self.screen = self.after_pairing.take().unwrap_or(Screen::Home);
                        } else {
                            self.trust_contact = Some(*contact);
                            self.trust_fingerprint.clear();
                        }
                    }
                    Payload::Trusted => {
                        self.screen = self.after_pairing.take().unwrap_or(Screen::Home);
                    }
                    Payload::ParcelExported => {}
                    Payload::ReceiptExported(id) => {
                        self.receipt_exported.insert(id);
                    }
                    Payload::ReceiptImported(record) => {
                        self.selected_delivery =
                            Some((record.parcel_id.clone(), Direction::Outgoing));
                        self.screen = Screen::Activity;
                    }
                    Payload::TransferCandidates(paths) => {
                        self.transfer_candidates = paths;
                        if self.transfer_candidates.len() == 1 {
                            let path = self.transfer_candidates[0].clone();
                            self.parcel_input = path.display().to_string();
                            self.start(context, Action::Inspect(path));
                        }
                    }
                    Payload::ReceiptCandidates(paths) => {
                        self.receipt_candidates = paths;
                        if self.receipt_candidates.len() == 1 {
                            let path = self.receipt_candidates[0].clone();
                            self.receipt_candidates.clear();
                            self.start(context, Action::ImportReceipt(path));
                        }
                    }
                    Payload::Measured(bytes) => self.selected_bytes = Some(bytes),
                    Payload::Sealed(details) => {
                        self.sealed_id = Some(details.record.parcel_id.clone());
                        self.selected_delivery =
                            Some((details.record.parcel_id.clone(), Direction::Outgoing));
                        self.sealed_inspection = Some(details.inspection);
                        self.sealed_file_bytes = Some(details.file_bytes);
                    }
                    Payload::Inspected(inspection) => {
                        self.inspection = Some(inspection);
                        self.notice.clear();
                    }
                    Payload::Received(result) => {
                        self.selected_delivery =
                            Some((result.record.parcel_id.clone(), Direction::Incoming));
                        self.received = Some(*result);
                    }
                    Payload::Recovered(summary) => self.recovery = Some(summary),
                    Payload::Discarded => self.recovery = None,
                    Payload::DemoUpdate { session, snapshot } => {
                        self.demo_session = Some(session);
                        self.demo_snapshot = Some(*snapshot);
                    }
                }
            }
        }
    }

    fn take_password(&mut self) -> Zeroizing<String> {
        Zeroizing::new(std::mem::take(&mut *self.password))
    }

    fn switch_screen(&mut self, screen: Screen) {
        if self.busy() || self.modal_open() {
            return;
        }
        if self.screen != screen {
            self.password.zeroize();
            self.confirmation.zeroize();
            self.notice.clear();
            self.error = None;
            self.screen = screen;
        }
    }

    fn header(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(192.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(RAIL)
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(context, |ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("RelayPost")
                        .font(display_font(24.0))
                        .color(Color32::WHITE),
                );
                ui.label(
                    RichText::new("Offline, together.")
                        .size(13.0)
                        .color(Color32::from_rgb(166, 198, 190)),
                );
                ui.add_space(36.0);
                for (screen, label) in [
                    (Screen::Home, "Home"),
                    (Screen::Send, "Send files"),
                    (Screen::Receive, "Receive files"),
                    (Screen::Activity, "Activity"),
                    (Screen::People, "People"),
                ] {
                    let selected = self.screen == screen;
                    let button =
                        egui::Button::new(RichText::new(label).font(display_font(16.0)).color(
                            if selected {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(187, 211, 205)
                            },
                        ))
                        .fill(if selected { TEAL } else { Color32::TRANSPARENT })
                        .min_size(egui::vec2(ui.available_width(), 44.0));
                    if ui
                        .add_enabled(!self.busy() && !self.modal_open(), button)
                        .clicked()
                    {
                        self.switch_screen(screen);
                    }
                    ui.add_space(5.0);
                }
                ui.add_space((ui.available_height() - 158.0).max(24.0));
                ui.label(
                    RichText::new("THIS COMPUTER")
                        .size(11.0)
                        .color(Color32::from_rgb(136, 171, 164)),
                );
                ui.label(
                    RichText::new(
                        self.identity
                            .as_ref()
                            .map(|c| c.name.as_str())
                            .unwrap_or("Not set up yet"),
                    )
                    .font(display_font(14.0))
                    .color(Color32::WHITE),
                );
                ui.add_space(16.0);
                if ui
                    .add_enabled(
                        !self.busy() && !self.modal_open(),
                        egui::Button::new(
                            RichText::new("Help & walkthrough")
                                .size(13.0)
                                .color(Color32::from_rgb(187, 211, 205)),
                        )
                        .fill(Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.switch_screen(Screen::Demo);
                }
            });
    }

    fn contact_name(&self, fingerprint: &str) -> String {
        if let Some(card) = self
            .identity
            .as_ref()
            .filter(|card| card.fingerprint == fingerprint)
        {
            return card.name.clone();
        }
        self.contacts
            .iter()
            .find(|contact| contact.card.fingerprint == fingerprint)
            .map(|contact| contact.card.name.clone())
            .unwrap_or_else(|| "Other computer".to_owned())
    }

    fn other_name(&self, record: &ParcelRecord) -> String {
        self.contact_name(if record.direction == Direction::Outgoing {
            &record.recipient
        } else {
            &record.sender
        })
    }

    fn connect_for(&mut self, destination: Screen) {
        self.after_pairing = Some(destination);
        self.switch_screen(Screen::People);
    }

    fn import_card(&mut self, context: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open their RelayPost connection card")
            .add_filter("Connection card", &["json"])
            .pick_file()
        {
            self.start(context, Action::ImportContact(path));
        }
    }

    fn import_confirmation(&mut self, context: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Choose the returned drive or confirmation folder")
            .pick_folder()
        {
            self.receipt_candidates.clear();
            self.start(context, Action::FindReceipts(path));
        }
    }

    fn choose_receive(&mut self, context: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Choose a drive or RelayPost transfer folder")
            .pick_folder()
        {
            self.inspect_selected(context, path);
        }
    }

    fn inspect_selected(&mut self, context: &egui::Context, path: PathBuf) {
        self.inspection = None;
        self.received = None;
        self.transfer_candidates.clear();
        self.parcel_input.clear();
        self.start(context, Action::FindTransfers(path));
    }

    fn messages(&mut self, ui: &mut Ui) {
        if let Some(pending) = &self.pending {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(RichText::new(pending.label).color(TEAL));
            });
            ui.add_space(16.0);
        }
        if let Some(error) = self.error.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(251, 231, 226))
                .inner_margin(egui::Margin::same(18))
                .corner_radius(12)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("This step needs your attention")
                            .font(display_font(18.0))
                            .color(RED),
                    );
                    ui.label(friendly_error(&error));
                    ui.collapsing("Technical details", |ui| {
                        ui.label(&error);
                    });
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            ui.add_space(20.0);
        } else if !self.notice.is_empty() {
            egui::Frame::new()
                .fill(Color32::from_rgb(223, 243, 236))
                .inner_margin(egui::Margin::same(16))
                .corner_radius(12)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&self.notice).color(GREEN));
                        if ui.small_button("Dismiss").clicked() {
                            self.notice.clear();
                        }
                    });
                });
            ui.add_space(20.0);
        }
    }

    fn send_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Send files",
            "Prepare a protected copy for another computer.",
        );
        if self.identity.is_none() {
            surface(ui, |ui| {
                ui.label(RichText::new("First, set up this computer").font(display_font(22.0)));
                ui.label("Give this computer a name and a password. No online account is needed.");
                ui.add_space(16.0);
                if primary(ui, "Set up this computer", true).clicked() {
                    self.connect_for(Screen::Send);
                }
            });
            return;
        }
        let trusted: Vec<_> = self
            .contacts
            .iter()
            .filter(|contact| {
                contact.verified
                    && self
                        .identity
                        .as_ref()
                        .is_none_or(|own| own.fingerprint != contact.card.fingerprint)
            })
            .cloned()
            .collect();
        if trusted.is_empty() {
            surface(ui, |ui| {
                ui.label(RichText::new("Connect the other computer").font(display_font(22.0)));
                ui.label("Exchange connection cards once, then check the complete safety codes together. This keeps your files going to the right person.");
                ui.add_space(16.0);
                if primary(ui, "Connect another computer", true).clicked() {
                    self.connect_for(Screen::Send);
                }
            });
            return;
        }
        let sealed = self
            .sealed_id
            .as_ref()
            .and_then(|id| {
                self.deliveries
                    .iter()
                    .find(|r| r.parcel_id == *id && r.direction == Direction::Outgoing)
            })
            .cloned();
        if let Some(record) = sealed {
            let name = self.other_name(&record);
            surface(ui, |ui| {
                let heading = match record.state {
                    DeliveryState::Ready => "Transfer ready",
                    DeliveryState::Confirmed => "Delivery confirmed",
                    DeliveryState::Cancelled => "Transfer stopped",
                    _ => "Ready to carry",
                };
                ui.label(RichText::new(heading).font(display_font(25.0)));
                ui.label(format!(
                    "To {name} · {} · {}",
                    file_count(record.files),
                    bytes(record.total_bytes)
                ));
                ui.add_space(16.0);
                draw_record(ui, &record);
                ui.collapsing("Protected copy details", |ui| {
                    if let Some(size) = self.sealed_file_bytes {
                        ui.label(format!("Stored copy: {}", bytes(size)));
                    }
                    if let Some(inspection) = &self.sealed_inspection {
                        draw_inspection(ui, inspection);
                    }
                });
                ui.add_space(16.0);
                match record.state {
                    DeliveryState::Ready => {
                        ui.label("Choose a USB drive or folder. RelayPost will copy the protected transfer there.");
                        if primary(ui, "Choose drive or folder…", true).clicked() {
                            self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                        }
                    }
                    DeliveryState::WaitingReceipt => {
                        ui.label(format!("Eject the drive and take it to {name}. Confirmation has not returned yet."));
                        if primary(ui, "Add confirmation from returned drive…", true).clicked() {
                            self.import_confirmation(ui.ctx());
                        }
                        if ui.button("Copy to another drive…").clicked() {
                            self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                        }
                    }
                    DeliveryState::Confirmed => {
                        ui.label(format!(
                            "{name} confirmed that the complete batch was saved correctly."
                        ));
                    }
                    _ => {}
                }
                if ui.button("Start another transfer").clicked() {
                    self.inputs.clear();
                    self.selected_bytes = Some(0);
                    self.invalidate_sealed();
                }
            });
            return;
        }
        if !trusted.iter().any(|c| c.card.fingerprint == self.recipient) {
            self.recipient = if trusted.len() == 1 {
                trusted[0].card.fingerprint.clone()
            } else {
                String::new()
            };
        }
        surface(ui, |ui| {
            section_label(ui, "TO");
            let selected = trusted
                .iter()
                .find(|c| c.card.fingerprint == self.recipient)
                .map(|c| c.card.name.as_str())
                .unwrap_or("Choose the other computer");
            egui::ComboBox::from_id_salt("recipient")
                .selected_text(selected)
                .width(ui.available_width().min(420.0))
                .show_ui(ui, |ui| {
                    for contact in &trusted {
                        if ui
                            .selectable_value(
                                &mut self.recipient,
                                contact.card.fingerprint.clone(),
                                &contact.card.name,
                            )
                            .changed()
                        {
                            self.invalidate_sealed();
                        }
                    }
                });
            if let Some(contact) = trusted
                .iter()
                .find(|c| c.card.fingerprint == self.recipient)
            {
                ui.label(
                    RichText::new("Safety code checked · ready to exchange")
                        .small()
                        .color(GREEN),
                );
                ui.collapsing("Connection details", |ui| {
                    fingerprint(ui, &contact.card.fingerprint);
                });
            }
        });
        ui.add_space(20.0);
        surface(ui, |ui| {
            section_label(ui, "FILES");
            ui.label(RichText::new("What would you like to send?").font(display_font(21.0)));
            ui.label(
                RichText::new("Drop files into this window, or choose them below.").color(MUTED),
            );
            ui.add_space(12.0);
            if ui.button("Choose files…").clicked() {
                if let Some(files) = rfd::FileDialog::new()
                    .set_title("Choose files to send")
                    .pick_files()
                {
                    for file in files {
                        if !self.inputs.contains(&file) {
                            self.inputs.push(file);
                        }
                    }
                    self.measure_inputs(ui.ctx());
                }
            }
            if !self.inputs.is_empty() {
                ui.add_space(12.0);
                let mut remove = None;
                egui::ScrollArea::vertical()
                    .id_salt("selected_files")
                    .max_height(190.0)
                    .show(ui, |ui| {
                        for (index, input) in self.inputs.iter().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    input
                                        .file_name()
                                        .map(|n| n.to_string_lossy())
                                        .unwrap_or_default(),
                                )
                                .on_hover_text(input.display().to_string());
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(index);
                                }
                            });
                        }
                    });
                if let Some(index) = remove {
                    self.inputs.remove(index);
                    self.measure_inputs(ui.ctx());
                }
                ui.add_space(8.0);
                ui.label(format!(
                    "{} · {}",
                    file_count(self.inputs.len()),
                    self.selected_bytes
                        .map(bytes)
                        .unwrap_or_else(|| "Checking size…".to_owned())
                ));
                if ui.small_button("Clear selection").clicked() {
                    self.inputs.clear();
                    self.selected_bytes = Some(0);
                    self.invalidate_sealed();
                }
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new("Up to 100 files · 64 MiB total · originals stay on this computer")
                    .small()
                    .color(MUTED),
            );
        });
        ui.add_space(20.0);
        let valid = !self.inputs.is_empty()
            && self.inputs.len() <= 100
            && self.selected_bytes.is_some_and(|n| n <= 64 * 1024 * 1024)
            && trusted.iter().any(|c| c.card.fingerprint == self.recipient);
        if self.inputs.len() > 100 {
            ui.label(RichText::new("Choose 100 files or fewer for one transfer.").color(RED));
        }
        if self
            .selected_bytes
            .is_some_and(|size| size > 64 * 1024 * 1024)
        {
            ui.label(
                RichText::new(
                    "This selection is too large. Remove files until the total is at most 64 MiB.",
                )
                .color(RED),
            );
        }
        if !self.recipient.is_empty() {
            ui.label(format!(
                "Only {} can open this protected copy. Damage recovery is included.",
                self.contact_name(&self.recipient)
            ));
        }
        if primary(ui, "Prepare transfer", valid).clicked() {
            self.unlock = Some(Unlock::Prepare);
        }
    }

    fn measure_inputs(&mut self, context: &egui::Context) {
        self.invalidate_sealed();
        self.selected_bytes = None;
        self.start(context, Action::Measure(self.inputs.clone()));
    }

    fn invalidate_sealed(&mut self) {
        self.sealed_id = None;
        self.sealed_inspection = None;
        self.sealed_file_bytes = None;
        self.notice.clear();
    }

    fn receive_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Receive files",
            "Bring a protected transfer from a drive onto this computer.",
        );
        if self.identity.is_none() {
            surface(ui, |ui| {
                ui.label(RichText::new("Set up this computer first").font(display_font(22.0)));
                ui.label("The sender needs this computer's connection card before they prepare files for you.");
                if primary(ui, "Set up this computer", true).clicked() {
                    self.connect_for(Screen::Receive);
                }
            });
            return;
        }
        if let Some(result) = self.received.clone() {
            if result.record.state == DeliveryState::Failed {
                surface(ui, |ui| {
                    ui.label(
                        RichText::new(if result.record.files == 1 {
                            "Saved file needs attention"
                        } else {
                            "Saved files need attention"
                        })
                        .font(display_font(24.0))
                        .color(RED),
                    );
                    ui.label("The earlier saved content could not be verified. Review local records before returning a confirmation.");
                    if primary(ui, "Review in Activity", true).clicked() {
                        self.selected_delivery =
                            Some((result.record.parcel_id.clone(), Direction::Incoming));
                        self.switch_screen(Screen::Activity);
                    }
                });
                return;
            }
            let name = self.other_name(&result.record);
            surface(ui, |ui| {
                ui.label(
                    RichText::new(if result.record.files == 1 {
                        "File saved on this computer"
                    } else {
                        "Files saved on this computer"
                    })
                    .font(display_font(25.0))
                    .color(GREEN),
                );
                ui.label(format!(
                    "From {name} · {} verified byte for byte",
                    file_count(result.record.files)
                ));
                draw_record(ui, &result.record);
                if result.repaired_shards > 0 {
                    ui.label(format!("Damaged pieces repaired: {}", result.repaired_shards));
                }
                ui.add_space(16.0);
                if self.receipt_exported.contains(&result.record.parcel_id) {
                    ui.label(format!(
                        "Confirmation is on the drive. Take it back to {name}."
                    ));
                } else {
                    ui.label(format!(
                        "Return a confirmation so {name} knows the transfer was saved correctly."
                    ));
                }
                if primary(
                    ui,
                    "Put confirmation on the drive…",
                    result.record.receipt_path.is_some(),
                )
                .clicked()
                {
                    self.choose_receipt_export(ui.ctx(), &result.record.parcel_id);
                }
                if let Some(path) = &result.record.received_path {
                    if ui.button("Show saved files").clicked() {
                        self.start(ui.ctx(), Action::OpenFolder(path.clone()));
                    }
                }
                if ui.button("Receive another transfer…").clicked() {
                    self.choose_receive(ui.ctx());
                }
            });
            return;
        }
        if let Some(inspection) = self.inspection.clone() {
            surface(ui, |ui| {
                ui.label(
                    RichText::new(format!("From {}", self.contact_name(&inspection.sender)))
                        .font(display_font(22.0)),
                );
                let missing: usize = inspection
                    .groups
                    .iter()
                    .map(|g| g.missing_shards.len())
                    .sum();
                if inspection.recoverable {
                    ui.label(
                        RichText::new("Ready to receive")
                            .font(display_font(24.0))
                            .color(GREEN),
                    );
                    ui.label(if missing == 0 {
                        "The sender and this copy were checked. Your files are ready to save."
                            .to_owned()
                    } else {
                        format!("Missing or damaged pieces: {missing}. This copy can be rebuilt automatically.")
                    });
                    ui.add_space(16.0);
                    if primary(ui, "Save files on this computer", true).clicked() {
                        self.unlock = Some(Unlock::Receive(PathBuf::from(&self.parcel_input)));
                    }
                } else {
                    ui.label(
                        RichText::new("This copy is too damaged")
                            .font(display_font(24.0))
                            .color(RED),
                    );
                    ui.label("Ask the sender to copy the prepared transfer to the drive again. This copy cannot be rebuilt.");
                    if primary(ui, "Choose another copy…", true).clicked() {
                        self.choose_receive(ui.ctx());
                    }
                }
                ui.add_space(12.0);
                ui.collapsing("Repair details", |ui| {
                    draw_inspection(ui, &inspection);
                });
                if inspection.recoverable && ui.button("Choose a different transfer…").clicked() {
                    self.choose_receive(ui.ctx());
                }
            });
            return;
        }
        surface(ui, |ui| {
            ui.label(RichText::new("Where are the files?").font(display_font(24.0)));
            ui.label("Plug in the drive, then choose it below. RelayPost will find transfer folders and check the copy for you.");
            ui.add_space(20.0);
            if primary(ui, "Choose drive or transfer folder…", true).clicked() {
                self.choose_receive(ui.ctx());
            }
            ui.label(
                RichText::new("You can also drop one drive or transfer folder into this window.")
                    .small()
                    .color(MUTED),
            );
        });
        if self.transfer_candidates.len() > 1 {
            ui.add_space(20.0);
            surface(ui, |ui| {
                ui.label(RichText::new("Choose a transfer to check").font(display_font(21.0)));
                ui.label("These folders have not been authenticated yet. The next step checks the sender and contents.");
                for (index, path) in self.transfer_candidates.clone().iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Transfer {}", index + 1));
                        ui.label(
                            RichText::new(short(
                                &path.file_name().unwrap_or_default().to_string_lossy(),
                            ))
                            .small()
                            .color(MUTED),
                        );
                        if ui.button("Check transfer").clicked() {
                            self.parcel_input = path.display().to_string();
                            self.start(ui.ctx(), Action::Inspect(path.clone()));
                        }
                    });
                }
            });
        }
    }

    fn activity_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Activity",
            "See what is ready, what needs carrying, and what is confirmed.",
        );
        let waiting_for_confirmation = self.deliveries.iter().any(|record| {
            record.direction == Direction::Outgoing && record.state == DeliveryState::WaitingReceipt
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh").clicked() {
                self.start(ui.ctx(), Action::Refresh);
            }
            if waiting_for_confirmation && ui.button("Add confirmation from drive…").clicked() {
                self.import_confirmation(ui.ctx());
            }
        });
        ui.add_space(20.0);
        if self.deliveries.is_empty() {
            surface(ui, |ui| {
                ui.label(RichText::new("Your first handoff starts here").font(display_font(22.0)));
                ui.label("Prepared transfers and files saved here will appear in this list.");
                if primary(ui, "Send files", true).clicked() {
                    self.switch_screen(Screen::Send);
                }
                if ui.button("Receive files").clicked() {
                    self.switch_screen(Screen::Receive);
                }
            });
        } else {
            surface(ui, |ui| {
                let records = self.deliveries.clone();
                egui::ScrollArea::vertical()
                    .id_salt("deliveries")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for record in &records {
                            let label = format!(
                                "{} {} · {} · {}",
                                if record.direction == Direction::Outgoing {
                                    "To"
                                } else {
                                    "From"
                                },
                                self.other_name(record),
                                file_count(record.files),
                                bytes(record.total_bytes)
                            );
                            let selected =
                                self.selected_delivery
                                    .as_ref()
                                    .is_some_and(|(id, direction)| {
                                        id == &record.parcel_id && direction == &record.direction
                                    });
                            ui.push_id(
                                (&record.parcel_id, format!("{:?}", record.direction)),
                                |ui| {
                                    if ui.selectable_label(selected, label).clicked() {
                                        self.selected_delivery = Some((
                                            record.parcel_id.clone(),
                                            record.direction.clone(),
                                        ));
                                    }
                                    state_badge(ui, &record.state);
                                    if record.local_error.is_some() {
                                        ui.label(
                                            RichText::new("Stored copy needs attention")
                                                .small()
                                                .color(RED),
                                        );
                                    }
                                    ui.separator();
                                },
                            );
                        }
                    });
            });
            if let Some(record) = self
                .selected_delivery
                .as_ref()
                .and_then(|(id, direction)| {
                    self.deliveries
                        .iter()
                        .find(|r| &r.parcel_id == id && &r.direction == direction)
                })
                .cloned()
            {
                ui.add_space(20.0);
                surface(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            if record.direction == Direction::Outgoing {
                                "To"
                            } else {
                                "From"
                            },
                            self.other_name(&record)
                        ))
                        .font(display_font(23.0)),
                    );
                    draw_record(ui, &record);
                    ui.add_space(12.0);
                    if record.direction == Direction::Outgoing
                        && record.state == DeliveryState::Ready
                        && primary(ui, "Copy to drive…", true).clicked()
                    {
                        self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                    }
                    if record.direction == Direction::Outgoing
                        && record.state == DeliveryState::WaitingReceipt
                        && primary(ui, "Add confirmation from returned drive…", true).clicked()
                    {
                        self.import_confirmation(ui.ctx());
                    }
                    if record.direction == Direction::Incoming
                        && record.state == DeliveryState::SavedReceiptReady
                        && record.receipt_path.is_some()
                        && primary(ui, "Put confirmation on the drive…", true).clicked()
                    {
                        self.choose_receipt_export(ui.ctx(), &record.parcel_id);
                    }
                    if let Some(path) = &record.received_path {
                        if ui.button("Show saved files").clicked() {
                            self.start(ui.ctx(), Action::OpenFolder(path.clone()));
                        }
                    }
                    ui.collapsing("More actions", |ui| {
                        if record.direction == Direction::Outgoing
                            && record.state != DeliveryState::Cancelled
                        {
                            if ui.button("Copy this transfer again…").clicked() {
                                self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                            }
                            if matches!(
                                record.state,
                                DeliveryState::Ready | DeliveryState::WaitingReceipt
                            ) && ui.button("Stop future copies…").clicked()
                            {
                                self.cancel_id = Some(record.parcel_id.clone());
                            }
                        }
                    });
                });
            }
        }
        ui.add_space(24.0);
        ui.collapsing("Recovery & local storage", |ui| {
            ui.label("If a save was interrupted, verify saved files and restore local records. Changed content is reported, never silently accepted.");
            if ui.button("Verify and recover…").clicked() { self.unlock=Some(Unlock::Recover); }
            if let Some(summary)=&self.recovery {
                ui.label(format!("{} recovered · {} unfinished · {} conflicts",summary.recovered,summary.pending,summary.conflicts.len()));
                for conflict in &summary.conflicts { ui.label(RichText::new(conflict).color(RED)); }
            }
            if ui.button("Discard unfinished temporary saves…").clicked() { self.discard_incomplete=true; }
        });
    }

    fn people_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "People",
            "Connect computers once. Exchange files whenever you need to.",
        );
        if self.identity.is_none() {
            surface(ui, |ui| {
                if !self.identity_missing {
                    ui.label(
                        RichText::new("This computer needs attention")
                            .font(display_font(22.0))
                            .color(RED),
                    );
                    ui.label("RelayPost could not read its existing setup. Restore an intact profile backup before using this identity again.");
                    if let Some(error) = &self.identity_error {
                        ui.collapsing("Technical details", |ui| {
                            ui.label(error);
                        });
                    }
                    if ui.button("Try reading it again").clicked() {
                        self.start(ui.ctx(), Action::Refresh);
                    }
                    return;
                }
                section_label(ui, "STEP 1 · THIS COMPUTER");
                ui.label(RichText::new("Give this computer a name").font(display_font(24.0)));
                ui.label("The other person will see this name. RelayPost keeps a private identity here so files can be protected for this computer.");
                ui.add_space(16.0);
                ui.label("Computer name");
                ui.add(
                    TextEdit::singleline(&mut self.identity_name)
                        .hint_text("e.g. Morgan's Mac")
                        .desired_width(ui.available_width()),
                );
                password_field(ui, &mut self.password);
                ui.label("Confirm password");
                ui.add(
                    TextEdit::singleline(&mut *self.confirmation)
                        .password(true)
                        .desired_width(ui.available_width()),
                );
                ui.label(RichText::new("This password protects RelayPost on this computer. There is no online account or password reset.").small().color(MUTED));
                ui.add_space(16.0);
                let valid = !self.identity_name.trim().is_empty()
                    && !self.password.is_empty()
                    && *self.password == *self.confirmation;
                if primary(ui, "Set up this computer", valid).clicked() {
                    let password = self.take_password();
                    self.confirmation.zeroize();
                    self.start(
                        ui.ctx(),
                        Action::Initialize(self.identity_name.trim().to_owned(), password),
                    );
                }
                if !self.confirmation.is_empty() && *self.password != *self.confirmation {
                    ui.label(RichText::new("Passwords do not match yet.").color(AMBER));
                }
            });
            self.profile_details(ui);
            return;
        }
        let card = self.identity.clone().expect("identity checked above");
        surface(ui, |ui| {
            section_label(ui, "THIS COMPUTER");
            ui.label(RichText::new(&card.name).font(display_font(24.0)));
            ui.label(RichText::new("Ready to connect").color(GREEN));
            if ui.button("Copy my safety code").clicked() {
                ui.ctx().copy_text(card.fingerprint.clone());
                self.notice = "Your complete safety code was copied. Share it through a separate channel you already trust.".to_owned();
            }
            ui.collapsing("Show my complete safety code", |ui| {
                fingerprint(ui, &card.fingerprint);
            });
            ui.collapsing("Technical connection details", |ui| {
                value(ui, "Encryption public key", &card.age_recipient);
                value(ui, "Signing public key", &card.signing_public_key);
            });
        });
        ui.add_space(20.0);
        surface(ui, |ui| {
            section_label(
                ui,
                if self.contacts.iter().any(|c| c.verified) {
                    "CONNECT ANOTHER COMPUTER"
                } else {
                    "STEP 2 · CONNECT THE OTHER COMPUTER"
                },
            );
            ui.label(RichText::new("Exchange connection cards").font(display_font(23.0)));
            ui.label("Both computers need RelayPost. Connect them once: give your connection card to the other person and ask for theirs. A card is public information and cannot open your files.");
            ui.add_space(16.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save my connection card…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Save your RelayPost connection card")
                        .set_file_name(connection_card_filename(&card.name, &card.fingerprint))
                        .add_filter("Connection card", &["json"])
                        .save_file()
                    {
                        self.start(ui.ctx(), Action::ExportCard(path));
                    }
                }
                if primary(ui, "Open their connection card…", true).clicked() {
                    self.import_card(ui.ctx());
                }
            });
            ui.label(RichText::new("After adding the cards, both people check the complete safety codes once. Future handoffs use the saved connection.").small().color(MUTED));
        });
        if !self.contacts.is_empty() {
            ui.add_space(20.0);
            surface(ui, |ui| {
                section_label(ui, "YOUR CONNECTIONS");
                for contact in self.contacts.clone() {
                    ui.push_id(&contact.card.fingerprint, |ui| {
                        ui.label(RichText::new(&contact.card.name).font(display_font(19.0)));
                        ui.label(
                            RichText::new(if contact.verified {
                                "Safety code checked · ready to exchange"
                            } else {
                                "Safety code not checked"
                            })
                            .color(if contact.verified {
                                GREEN
                            } else {
                                AMBER
                            }),
                        );
                        if !contact.verified && ui.button("Check safety code…").clicked() {
                            self.trust_contact = Some(contact.clone());
                            self.trust_fingerprint.clear();
                        }
                        ui.collapsing("Connection details", |ui| {
                            fingerprint(ui, &contact.card.fingerprint);
                        });
                        ui.add_space(12.0);
                    });
                }
                if self.contacts.iter().any(|c| c.verified)
                    && primary(ui, "Continue to files", true).clicked()
                {
                    let target = self.after_pairing.take().unwrap_or(Screen::Home);
                    self.switch_screen(target);
                }
            });
        }
        self.profile_details(ui);
    }

    fn profile_details(&self, ui: &mut Ui) {
        ui.add_space(24.0);
        ui.collapsing("Local storage details", |ui| {
            value(ui,"RelayPost data on this computer",&self.home.display().to_string());
            ui.label("To use this exact storage with the CLI, pass --home followed by this directory. Keep a safe backup; it contains the password-protected private identity.");
        });
    }

    fn home_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Files delivered. Even offline.",
            "Prepare here. Carry by drive. Know when the other computer saved your files.",
        );
        surface(ui, |ui| {
            section_label(ui, "NEXT STEP");
            if let Some(record) = self
                .deliveries
                .iter()
                .find(|record| {
                    record.local_error.is_some() || record.state == DeliveryState::Failed
                })
                .cloned()
            {
                ui.label(
                    RichText::new("A local copy needs attention")
                        .font(display_font(25.0))
                        .color(RED),
                );
                ui.label(if record.state == DeliveryState::Confirmed { "The delivery remains confirmed, but the stored copy on this computer needs a check." } else { "Review the affected handoff before copying files or returning a confirmation." });
                ui.add_space(16.0);
                if primary(ui, "Review in Activity", true).clicked() {
                    self.selected_delivery =
                        Some((record.parcel_id.clone(), record.direction.clone()));
                    self.switch_screen(Screen::Activity);
                }
            } else if self.identity.is_none() {
                ui.label(RichText::new("Make this computer yours").font(display_font(25.0)));
                ui.label("Start with a name and password. Then connect the computer you exchange files with.");
                ui.add_space(20.0);
                if primary(ui, "Set up this computer", true).clicked() {
                    self.connect_for(Screen::Home);
                }
                ui.label(
                    RichText::new("No cloud account. Your files stay between the two computers.")
                        .small()
                        .color(MUTED),
                );
            } else if !self.contacts.iter().any(|c| c.verified) {
                ui.label(RichText::new("Connect the other computer").font(display_font(25.0)));
                ui.label("Exchange connection cards once and compare the complete safety codes together.");
                ui.add_space(20.0);
                if primary(ui, "Connect another computer", true).clicked() {
                    self.connect_for(Screen::Home);
                }
            } else if let Some(record) = self
                .deliveries
                .iter()
                .find(|r| r.direction == Direction::Outgoing && r.state == DeliveryState::Ready)
                .cloned()
            {
                ui.label(
                    RichText::new(format!(
                        "Your transfer to {} is ready",
                        self.other_name(&record)
                    ))
                    .font(display_font(23.0)),
                );
                ui.label(format!(
                    "{} · {} · choose a drive to carry this protected copy.",
                    file_count(record.files),
                    bytes(record.total_bytes)
                ));
                ui.add_space(16.0);
                if primary(ui, "Copy prepared files to a drive…", true).clicked() {
                    self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                }
            } else if let Some(record) = self
                .deliveries
                .iter()
                .find(|r| {
                    r.direction == Direction::Incoming
                        && r.state == DeliveryState::SavedReceiptReady
                        && !self.receipt_exported.contains(&r.parcel_id)
                })
                .cloned()
            {
                ui.label(
                    RichText::new(format!(
                        "Let {} know the files arrived",
                        self.other_name(&record)
                    ))
                    .font(display_font(23.0)),
                );
                ui.label("The files are saved here. Put their confirmation on the drive and carry it back.");
                ui.add_space(16.0);
                if primary(ui, "Put confirmation on the drive…", true).clicked() {
                    self.choose_receipt_export(ui.ctx(), &record.parcel_id);
                }
            } else if let Some(record) = self
                .deliveries
                .iter()
                .find(|r| {
                    r.direction == Direction::Outgoing && r.state == DeliveryState::WaitingReceipt
                })
                .cloned()
            {
                ui.label(
                    RichText::new(format!(
                        "Waiting for {}'s confirmation",
                        self.other_name(&record)
                    ))
                    .font(display_font(23.0)),
                );
                ui.label("Copied to a drive. When the drive comes back, add its confirmation to finish the handoff.");
                ui.add_space(16.0);
                if primary(ui, "Add confirmation from returned drive…", true).clicked() {
                    self.import_confirmation(ui.ctx());
                }
            } else {
                ui.label(RichText::new("Ready for your next handoff").font(display_font(25.0)));
                ui.label("Send a protected copy, or bring files from a drive onto this computer.");
                ui.add_space(20.0);
                ui.horizontal_wrapped(|ui| {
                    if primary(ui, "Send files", true).clicked() {
                        self.switch_screen(Screen::Send);
                    }
                    if ui.button("Receive files").clicked() {
                        self.switch_screen(Screen::Receive);
                    }
                });
            }
        });
        ui.add_space(24.0);
        ui.columns(2, |columns| {
            surface(&mut columns[0], |ui| {
                ui.label(RichText::new("Send files").font(display_font(22.0)));
                ui.label("Prepare a protected copy for someone else.");
                ui.add_space(12.0);
                if ui.button("Start sending").clicked() {
                    self.switch_screen(Screen::Send);
                }
            });
            surface(&mut columns[1], |ui| {
                ui.label(RichText::new("Receive files").font(display_font(22.0)));
                ui.label("Check and save a transfer from a drive.");
                ui.add_space(12.0);
                if ui.button("Start receiving").clicked() {
                    self.switch_screen(Screen::Receive);
                }
            });
        });
        if !self.deliveries.is_empty() {
            ui.add_space(24.0);
            section_label(ui, "RECENT HANDOFFS");
            for record in self.deliveries.clone().iter().rev().take(3) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} {} · {}",
                        if record.direction == Direction::Outgoing {
                            "To"
                        } else {
                            "From"
                        },
                        self.other_name(record),
                        file_count(record.files)
                    ));
                    state_badge(ui, &record.state);
                    if ui.small_button("View").clicked() {
                        self.selected_delivery =
                            Some((record.parcel_id.clone(), record.direction.clone()));
                        self.switch_screen(Screen::Activity);
                    }
                });
            }
        }
        ui.add_space(24.0);
        if ui
            .link("New to RelayPost? Try the guided walkthrough")
            .clicked()
        {
            let ctx = ui.ctx().clone();
            self.start_demo(&ctx);
        }
        ui.label(
            RichText::new("The walkthrough uses sample files and separate test computers.")
                .small()
                .color(MUTED),
        );
    }

    fn start_demo(&mut self, context: &egui::Context) {
        if self.busy() {
            return;
        }
        // Compute a unique temp root; never reuse or delete old ones.
        let pid = std::process::id();
        let nanos = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(error) => {
                self.error = Some(format!("Cannot name a new demo directory: {error}. Correct the system clock and try again."));
                return;
            }
        };
        let temp = match std::fs::canonicalize(std::env::temp_dir()) {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(format!("Cannot locate the temporary directory for a new demo: {error}. Check the system's temporary directory and try again."));
                return;
            }
        };
        let root = temp.join(format!("relaypost-demo-{pid}-{nanos}"));
        // Clear old snapshot/session only when not busy (already guaranteed above).
        self.demo_snapshot = None;
        self.demo_session = None;
        self.switch_screen(Screen::Demo);
        self.start(context, Action::DemoStart(root));
    }

    fn demo_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Guided walkthrough",
            "One Mac, two synthetic devices, a directory as the carrier. No user files or physical USB required.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("2 generated files / 21 MiB (20 + 1 MiB)").color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Start a new walkthrough"))
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    self.start_demo(&ctx);
                }
            });
        });
        ui.add_space(8.0);

        // Draw public facts only; session ownership stays with the worker during advances.
        let Some(snap) = self.demo_snapshot.clone() else {
            empty_state(ui, "Try an offline handoff", "Run a demo, damage its carrier copy, repair and receive the files, then return Bob's signed receipt to Alice.");
            return;
        };
        let phase_number = match snap.phase {
            DemoPhase::Prepared => 0,
            DemoPhase::Damaged => 1,
            DemoPhase::Received => 2,
            DemoPhase::Confirmed => 3,
        };
        ui.horizontal_wrapped(|ui| {
            for (index, label) in ["1  Prepared", "2  Damaged", "3  Received", "4  Confirmed"]
                .iter()
                .enumerate()
            {
                if index > 0 {
                    ui.label(RichText::new("->").color(MUTED));
                }
                let color = if index == phase_number {
                    TEAL
                } else if index < phase_number {
                    GREEN
                } else {
                    MUTED
                };
                ui.label(RichText::new(*label).strong().color(color));
            }
        });
        ui.add_space(8.0);

        // Keep the next action above all details so the demo works without scrolling.
        let ctx = ui.ctx().clone();
        let can_advance = !self.busy() && self.demo_session.is_some();
        let next = match snap.phase {
            DemoPhase::Prepared => Some(("Damage carrier copy ->", "Delete 2 data shards, corrupt 2 parity shards, and prove that a fifth erasure exceeds the repair budget.")),
            DemoPhase::Damaged => Some(("Repair and receive ->", "Bob reconstructs the four unavailable shards, verifies both files, and creates a signed receipt.")),
            DemoPhase::Received => Some(("Carry back signed receipt ->", "Export Bob's receipt to the carrier, then let Alice verify it and confirm delivery.")),
            DemoPhase::Confirmed => None,
        };
        if let Some((label, description)) = next {
            if primary(ui, label, can_advance).clicked() {
                if let Some(session) = self.demo_session.take() {
                    self.start(&ctx, Action::DemoAdvance(session));
                }
            }
            ui.label(RichText::new(description).small().color(MUTED));
        } else {
            ui.label(
                RichText::new("Delivery confirmed by Bob's signed receipt.")
                    .strong()
                    .color(GREEN),
            );
            if let Some(report_src) = snap.report_path.clone() {
                if primary(ui, "Save walkthrough evidence…", can_advance).clicked() {
                    if let Some(target) = rfd::FileDialog::new()
                        .set_file_name("relaypost-demo-report.json")
                        .save_file()
                    {
                        self.start(&ctx, Action::DemoCopyReport(report_src, target));
                    }
                }
                ui.label(
                    RichText::new(
                        "The JSON evidence report is unsigned. The delivery receipt is signed.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
        }
        if !self.busy() && self.demo_session.is_none() {
            ui.label(RichText::new("This demo session stopped. These are its last successful results. Run a new guided demo to continue.").color(RED));
        }
        ui.add_space(10.0);

        surface(ui, |ui| {
            ui.columns(2, |columns| {
                columns[0].label(
                    RichText::new("ALICE / SENDER")
                        .small()
                        .strong()
                        .color(MUTED),
                );
                if let Some(state) = &snap.sender_state {
                    state_badge(&mut columns[0], state);
                }
                columns[1].label(
                    RichText::new("BOB / RECIPIENT")
                        .small()
                        .strong()
                        .color(MUTED),
                );
                if let Some(repaired) = snap.repaired_shards {
                    columns[1].label(
                        RichText::new(format!("Batch saved / {repaired} shards repaired"))
                            .strong()
                            .color(GREEN),
                    );
                    columns[1].label(if snap.phase == DemoPhase::Confirmed {
                        "Signed receipt returned to Alice"
                    } else {
                        "Signed receipt ready for the return trip"
                    });
                } else {
                    columns[1].label("Awaiting verified files");
                }
            });
            if !snap.source_sha256.is_empty() && snap.source_sha256 == snap.received_sha256 {
                ui.label(
                    RichText::new(format!(
                        "MATCH / SHA-256 verified for all {}",
                        file_count(snap.source_sha256.len())
                    ))
                    .strong()
                    .color(GREEN),
                );
            } else if !snap.received_sha256.is_empty() {
                ui.label(
                    RichText::new("MISMATCH / source and received SHA-256 maps differ")
                        .strong()
                        .color(RED),
                );
            }
        });
        ui.add_space(10.0);

        // Receiving repairs Bob's private output, never this damaged carrier copy.
        let carrier_inspection = snap
            .inspection_after_damage
            .as_ref()
            .or(snap.initial_inspection.as_ref());
        if let Some(inspection) = carrier_inspection {
            surface(ui, |ui| {
                ui.label(
                    RichText::new(if snap.inspection_after_damage.is_some() {
                        "Repair details"
                    } else {
                        "Carrier inspection before damage"
                    })
                    .strong(),
                );
                for group in &inspection.groups {
                    let missing_data = group
                        .missing_shards
                        .iter()
                        .filter(|&&index| index < group.data_shards)
                        .count();
                    let missing_parity = group
                        .missing_shards
                        .iter()
                        .filter(|&&index| index >= group.data_shards)
                        .count();
                    ui.label(format!(
                        "Group {} / {}/{} data + {}/{} parity / {} unavailable / {}",
                        group.index,
                        group.data_shards - missing_data,
                        group.data_shards,
                        group.parity_shards - missing_parity,
                        group.parity_shards,
                        group.missing_shards.len(),
                        if group.recoverable {
                            "recoverable"
                        } else {
                            "over budget"
                        },
                    ));
                    ui.horizontal_wrapped(|ui| {
                        for index in 0..group.data_shards + group.parity_shards {
                            let missing = group.missing_shards.contains(&index);
                            let parity = index >= group.data_shards;
                            let fill = if missing {
                                Color32::from_rgb(226, 173, 119)
                            } else if parity {
                                Color32::from_rgb(111, 155, 174)
                            } else {
                                Color32::from_rgb(81, 155, 137)
                            };
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(23.0, 26.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect.shrink(1.0), 3, fill);
                            response.on_hover_text(format!(
                                "Group {} / shard {index} / {} / {}",
                                group.index,
                                if parity { "parity" } else { "data" },
                                if missing {
                                    "missing or hash mismatch"
                                } else {
                                    "digest verified"
                                },
                            ));
                        }
                    });
                }
                ui.label(RichText::new("Green: verified data   Blue: verified parity   Amber: missing / hash mismatch").small().color(MUTED));
                if snap.received_path.is_some() {
                    ui.label(RichText::new("Carrier evidence remains damaged. Bob repaired his private received output.").small().color(MUTED));
                }
            });
            ui.add_space(8.0);
        }

        if let Some(inspection) = &snap.five_erasure_refusal_evidence {
            surface(ui, |ui| {
                let refused = snap.five_erasure_receive_error.is_some() && !inspection.recoverable;
                ui.label(
                    RichText::new(if refused {
                        "5-erasure receive refused / over budget"
                    } else {
                        "5-erasure inspection evidence"
                    })
                    .strong()
                    .color(if refused { RED } else { MUTED }),
                );
                for group in &inspection.groups {
                    ui.label(format!(
                        "Group {}: {} unavailable / repair budget {} / {}",
                        group.index,
                        group.missing_shards.len(),
                        group.parity_shards,
                        if group.recoverable {
                            "recoverable"
                        } else {
                            "over budget"
                        }
                    ));
                }
                ui.label(RichText::new("Shard 2 was restored after this attempt, leaving four erasures for the next receive.").small().color(MUTED));
                if let Some(error) = &snap.five_erasure_receive_error {
                    ui.collapsing("Full refusal error", |ui| {
                        ui.label(RichText::new(error).color(RED));
                    });
                }
            });
            ui.add_space(8.0);
        }

        egui::CollapsingHeader::new("File verification details")
            .id_salt(("demo_hashes", &snap.root, phase_number))
            .default_open(false)
            .show(ui, |ui| {
                for (name, source) in &snap.source_sha256 {
                    ui.label(RichText::new(name).strong());
                    value(ui, "Source SHA-256", source);
                    if let Some(received) = snap.received_sha256.get(name) {
                        value(ui, "Received SHA-256", received);
                        ui.label(
                            RichText::new(if source == received {
                                "MATCH"
                            } else {
                                "MISMATCH"
                            })
                            .color(if source == received {
                                GREEN
                            } else {
                                RED
                            }),
                        );
                    } else {
                        ui.label(RichText::new("Not received yet").color(MUTED));
                    }
                }
            });
        ui.collapsing("Demo files and evidence", |ui| {
            value(
                ui,
                "Demo root (old runs are preserved)",
                &snap.root.display().to_string(),
            );
            if let Some(path) = &snap.carrier_parcel_path {
                value(ui, "Carrier parcel", &path.display().to_string());
            }
            if let Some(path) = &snap.received_path {
                value(ui, "Received batch", &path.display().to_string());
            }
            if let Some(path) = &snap.receipt_path {
                value(ui, "Signed receipt", &path.display().to_string());
            }
            if let Some(path) = &snap.report_path {
                value(
                    ui,
                    "JSON evidence report (unsigned)",
                    &path.display().to_string(),
                );
            }
        });
    }

    fn choose_parcel_export(&mut self, context: &egui::Context, id: &str) {
        if let Some(carrier) = rfd::FileDialog::new()
            .set_title("Choose a USB drive or folder for this transfer")
            .pick_folder()
        {
            self.start(context, Action::ExportParcel(id.to_owned(), carrier));
        }
    }

    fn choose_receipt_export(&mut self, context: &egui::Context, id: &str) {
        if let Some(carrier) = rfd::FileDialog::new()
            .set_title("Choose a drive or folder for the confirmation")
            .pick_folder()
        {
            self.start(context, Action::ExportReceipt(id.to_owned(), carrier));
        }
    }

    fn dialogs(&mut self, context: &egui::Context) {
        if let Some(unlock) = self.unlock.clone() {
            let mut close = false;
            egui::Window::new("Unlock RelayPost").collapsible(false).resizable(false).default_width(440.0).show(context, |ui| {
                ui.label("Enter the password you set for this computer. It is used only for this step.");
                password_field(ui,&mut self.password);
                let label=match unlock { Unlock::Prepare=>"Prepare protected transfer", Unlock::Receive(_)=>"Save files on this computer", Unlock::Recover=>"Verify and recover" };
                ui.horizontal_wrapped(|ui| {
                    if primary(ui,label,!self.password.is_empty()).clicked() {
                        let password=self.take_password();
                        let action=match unlock.clone() {
                            Unlock::Prepare=>Action::Seal(self.inputs.clone(),self.recipient.clone(),password),
                            Unlock::Receive(path)=>Action::Receive(path,password),
                            Unlock::Recover=>Action::Recover(password),
                        };
                        self.start(context,action); close=true;
                    }
                    if ui.button("Cancel").clicked() { self.password.zeroize(); close=true; }
                });
            });
            if close {
                self.unlock = None;
            }
        }
        if self.receipt_candidates.len() > 1 {
            let mut close = false;
            egui::Window::new("Choose a returned confirmation").collapsible(false).resizable(false).default_width(480.0).show(context, |ui| {
                ui.label("RelayPost found several confirmation folders. Each one is verified before it can confirm a delivery.");
                for (index,path) in self.receipt_candidates.clone().iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Confirmation {}",index+1));
                        ui.label(RichText::new(short(&path.file_name().unwrap_or_default().to_string_lossy())).small().color(MUTED));
                        if ui.add_enabled(!self.busy(),egui::Button::new("Verify")).clicked() { self.start(context,Action::ImportReceipt(path.clone())); close=true; }
                    });
                }
                if ui.button("Cancel").clicked() { close=true; }
            });
            if close {
                self.receipt_candidates.clear();
            }
        }
        if let Some(contact) = self.trust_contact.clone() {
            let mut close = false;
            egui::Window::new("Check the complete safety code")
                .collapsible(false)
                .resizable(false)
                .default_width(570.0)
                .show(context, |ui| {
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        ui.label(format!(
                            "Ask {} to open People on their computer and click Copy my safety code. Get that complete code through a separate channel you already trust, then paste it below. Compare every group with the code shown here.",
                            contact.card.name
                        ));
                        fingerprint(ui, &contact.card.fingerprint);
                        ui.add_space(10.0);
                        ui.label("Paste the complete safety code from their computer (spaces are fine)");
                        ui.add(
                            TextEdit::singleline(&mut self.trust_fingerprint)
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(10.0);
                        ui.horizontal_wrapped(|ui| {
                            if primary(
                                ui,
                                "The complete code matches",
                                self.trust_fingerprint.split_whitespace().collect::<String>() == contact.card.fingerprint,
                            )
                            .clicked()
                            {
                                self.start(
                                    context,
                                    Action::Trust(contact.card.fingerprint.clone()),
                                );
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                });
            if close {
                self.trust_contact = None;
                self.trust_fingerprint.clear();
            }
        }
        if let Some(id) = self.cancel_id.clone() {
            let mut close = false;
            egui::Window::new("Stop future copies?").collapsible(false).resizable(false).default_width(460.0).show(context, |ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    ui.label("This stops local re-export of this parcel. It cannot recall copies already on a carrier and does not delete the original source files.");
                    value(ui, "Parcel", &id);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(RichText::new("Stop future copies").color(RED)).clicked() {
                            self.start(context, Action::Cancel(id.clone()));
                            close = true;
                        }
                        if ui.button("Keep delivery").clicked() { close = true; }
                    });
                });
            });
            if close {
                self.cancel_id = None;
            }
        }
        if self.discard_incomplete {
            let mut close = false;
            egui::Window::new("Discard interrupted receives?")
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .show(context, |ui| {
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        ui.label("Delete unfinished temporary receive data in this profile. Import the parcel again from your carrier when you want to receive it.");
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(RichText::new("Discard interrupted work").color(RED)).clicked() {
                                self.start(context, Action::DiscardIncomplete);
                                close = true;
                            }
                            if ui.button("Keep temporary work").clicked() {
                                close = true;
                            }
                        });
                    });
                });
            if close {
                self.discard_incomplete = false;
            }
        }
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        if !self.busy()
            && self.modal_open()
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.password.zeroize();
            self.unlock = None;
            self.trust_contact = None;
            self.trust_fingerprint.clear();
            self.cancel_id = None;
            self.discard_incomplete = false;
            self.receipt_candidates.clear();
        }
        for (key, screen) in [
            (egui::Key::Num1, Screen::Home),
            (egui::Key::Num2, Screen::Send),
            (egui::Key::Num3, Screen::Receive),
            (egui::Key::Num4, Screen::Activity),
            (egui::Key::Num5, Screen::People),
        ] {
            if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, key)) {
                self.switch_screen(screen);
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::R))
            && !self.busy()
            && !self.modal_open()
        {
            self.start(context, Action::Refresh);
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
            self.switch_screen(Screen::Demo);
        }
    }
}

impl eframe::App for RelayApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(context);
        self.shortcuts(context);
        if !self.busy() && !self.modal_open() {
            let dropped: Vec<_> = context.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.clone())
                    .collect()
            });
            if !dropped.is_empty() && self.screen == Screen::Send && self.sealed_id.is_none() {
                for file in dropped {
                    if !self.inputs.contains(&file) {
                        self.inputs.push(file);
                    }
                }
                self.measure_inputs(context);
            } else if dropped.len() == 1 && self.screen == Screen::Receive {
                self.inspect_selected(context, dropped[0].clone());
            }
        }
        self.header(context);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::symmetric(32, 28)),
            )
            .show(context, |ui| {
                let content_width = ui.available_width().min(860.0);
                egui::ScrollArea::vertical()
                    .id_salt("page")
                    .max_width(content_width)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                    // A maximum is an assignment, not a clamp: never widen a
                    // narrow viewport to the preferred large-window width.
                    ui.set_max_width(ui.available_width().min(content_width));
                    self.messages(ui);
                    ui.add_enabled_ui(!self.busy() && !self.modal_open(), |ui| match self.screen {
                        Screen::Home => self.home_screen(ui),
                        Screen::Send => self.send_screen(ui),
                        Screen::Receive => self.receive_screen(ui),
                        Screen::Activity => self.activity_screen(ui),
                        Screen::People => self.people_screen(ui),
                        Screen::Demo => self.demo_screen(ui),
                    });
                    ui.add_space(20.0);
                });
            });
        self.dialogs(context);
        if self.busy() {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn password_field(ui: &mut Ui, password: &mut Zeroizing<String>) {
    ui.add_space(5.0);
    ui.label("Password for this computer");
    ui.add(
        TextEdit::singleline(&mut **password)
            .password(true)
            .desired_width(ui.available_width())
            .hint_text("Your RelayPost password"),
    );
    ui.add_space(5.0);
}

fn display_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(DISPLAY_FAMILY.into()))
}

fn fingerprint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new("Complete safety code").small().color(MUTED));
    let groups = text
        .as_bytes()
        .chunks(8)
        .map(|part| String::from_utf8_lossy(part))
        .collect::<Vec<_>>()
        .join(" ");
    ui.label(RichText::new(groups).monospace().size(15.0));
}

fn value(ui: &mut Ui, label: &str, text: &str) {
    if !label.is_empty() {
        ui.label(RichText::new(label).small().color(MUTED));
    }
    let mut text = text;
    ui.add(
        TextEdit::multiline(&mut text)
            .font(egui::TextStyle::Monospace)
            .desired_rows(1)
            .desired_width(ui.available_width())
            .frame(false),
    );
}

fn draw_record(ui: &mut Ui, record: &ParcelRecord) {
    state_badge(ui, &record.state);
    ui.label(state_description(&record.state));
    if let Some(error) = &record.local_error {
        ui.label(
            RichText::new(
                "The stored copy needs attention. Delivery status above has not changed.",
            )
            .color(RED),
        );
        ui.collapsing("Stored copy details", |ui| {
            ui.label(error);
        });
    }
    let stage = match record.state {
        DeliveryState::Ready => Some(0),
        DeliveryState::WaitingReceipt => Some(1),
        DeliveryState::SavedReceiptReady => Some(2),
        DeliveryState::Confirmed => Some(3),
        _ => None,
    };
    if let Some(stage) = stage {
        handoff_ribbon(ui, stage);
    }
    ui.label(format!(
        "{} · {}",
        file_count(record.files),
        bytes(record.total_bytes)
    ));
    ui.collapsing("Technical record",|ui| {
        value(ui,"Transfer ID",&record.parcel_id);
        value(ui,"Sender safety code",&record.sender);
        value(ui,"Recipient safety code",&record.recipient);
        ui.label(format!("Created here: {}",record.created_at));
        ui.label(format!("Updated here: {}",record.updated_at));
        if let Some(path)=&record.parcel_path {value(ui,"Protected copy",&path.display().to_string());}
        if let Some(path)=&record.received_path {value(ui,"Saved files",&path.display().to_string());}
        if let Some(path)=&record.receipt_path {value(ui,"Signed confirmation",&path.display().to_string());}
        ui.label("A confirmation proves the verified save. It does not prove a person read the files or guarantee future storage.");
    });
}

fn draw_inspection(ui: &mut Ui, inspection: &Inspection) {
    for group in &inspection.groups {
        ui.label(format!(
            "Group {} · {} missing or damaged · repair limit {}",
            group.index,
            group.missing_shards.len(),
            group.parity_shards
        ));
        ui.horizontal_wrapped(|ui| {
            for index in 0..group.data_shards + group.parity_shards {
                let missing = group.missing_shards.contains(&index);
                let parity = index >= group.data_shards;
                let color = if missing {
                    AMBER
                } else if parity {
                    Color32::from_rgb(111, 155, 174)
                } else {
                    GREEN
                };
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::hover());
                ui.painter().rect_filled(rect.shrink(1.0), 3, color);
                response.on_hover_text(format!(
                    "Piece {index} · {}",
                    if missing {
                        "missing or damaged"
                    } else {
                        "digest checked"
                    }
                ));
            }
        });
    }
    ui.label(
        RichText::new("Green: healthy · blue: recovery pieces · amber: missing or damaged")
            .small()
            .color(MUTED),
    );
    value(ui, "Transfer ID", &inspection.parcel_id);
    value(ui, "Sender", &inspection.sender);
    value(ui, "Recipient", &inspection.recipient);
    ui.label(format!(
        "Protected content: {}",
        bytes(inspection.ciphertext_bytes)
    ));
}

fn friendly_error(error: &str) -> &'static str {
    let error = error.to_lowercase();
    if error.contains("prepared for another computer") {
        "This transfer is for a different computer. Ask the sender to prepare a new one using this computer's connection card."
    } else if error.contains("password") || error.contains("decrypt") {
        "Check the password for this computer and try again. The files have not been accepted by this failed step."
    } else if error.contains("signature") || error.contains("digest") || error.contains("tamper") {
        "This copy could not be authenticated. Ask the sender for a fresh copy before continuing."
    } else if error.contains("trust") || error.contains("contact") || error.contains("sender") {
        "Open People and connect the sending computer. Both computers must check each other's complete safety codes."
    } else if error.contains("repair") || error.contains("shard") {
        "This copy could not be rebuilt. Ask the sender to copy the prepared transfer to the drive again."
    } else if error.contains("regular")
        || error.contains("symlink")
        || error.contains("size")
        || error.contains("limit")
    {
        "Check the selected files or folder. Use ordinary files within the displayed limits, then choose them again."
    } else if error.contains("found") || error.contains("directory") || error.contains("folder") {
        "Choose the drive or folder that contains the RelayPost transfer or confirmation. Check that the drive is still connected."
    } else {
        "Review the details below before retrying. If this happened during a save, Activity can verify and recover local records."
    }
}

fn state_badge(ui: &mut Ui, state: &DeliveryState) {
    let (label, color) = match state {
        DeliveryState::Ready => ("Prepared", TEAL),
        DeliveryState::WaitingReceipt => ("Waiting for confirmation", AMBER),
        DeliveryState::SavedReceiptReady => ("Saved on this computer", GREEN),
        DeliveryState::Confirmed => ("Delivery confirmed", GREEN),
        DeliveryState::Failed => ("Needs attention", RED),
        DeliveryState::Cancelled => ("Future copies stopped", MUTED),
    };
    ui.label(RichText::new(label).font(display_font(15.0)).color(color));
}

fn state_description(state: &DeliveryState) -> &'static str {
    match state {
        DeliveryState::Ready => "The protected copy is ready to put on a drive.",
        DeliveryState::WaitingReceipt => {
            "Copied to a drive. The other computer has not confirmed the save yet."
        }
        DeliveryState::SavedReceiptReady => {
            "All files were verified and saved here. Return the confirmation to the sender."
        }
        DeliveryState::Confirmed => {
            "The intended computer confirmed that all files were saved correctly."
        }
        DeliveryState::Failed => {
            "The saved content needs attention. Review the details before trying again."
        }
        DeliveryState::Cancelled => {
            "This computer will make no more copies. Copies already carried away still work."
        }
    }
}

fn connection_card_filename(device_name: &str, fingerprint: &str) -> String {
    let mut name = String::new();
    for ch in device_name.chars().take(40) {
        if ch.is_alphanumeric() {
            name.push(ch);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_end_matches('-');
    let name = if name.is_empty() { "computer" } else { name };
    // This short suffix distinguishes file names; trust still uses the full code.
    let code: String = fingerprint
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .take(12)
        .collect();
    format!("{name}-{code}-relaypost-card.json")
}

fn file_count(value: usize) -> String {
    format!("{value} {}", if value == 1 { "file" } else { "files" })
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.2} MiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} bytes")
    }
}

fn short(value: &str) -> String {
    if value.chars().count() > 18 {
        format!("{}…", value.chars().take(18).collect::<String>())
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::connection_card_filename;

    #[test]
    fn connection_card_names_stay_single_bounded_file_names() {
        for name in ["../Alice\\Mac\0\n", "///", "小明的电脑", &"A".repeat(1000)] {
            let filename = connection_card_filename(name, &"a1".repeat(32));
            assert!(!filename.starts_with('.'));
            assert!(!filename.contains(['/', '\\']));
            assert!(!filename.chars().any(char::is_control));
            assert!(filename.len() <= 200);
            assert_eq!(std::path::Path::new(&filename).components().count(), 1);
            assert!(filename.ends_with("-relaypost-card.json"));
        }
    }

    #[test]
    fn same_named_computers_have_distinct_card_defaults() {
        let alice = connection_card_filename("My Mac", &"a".repeat(64));
        let bob = connection_card_filename("My Mac", &"b".repeat(64));
        assert_ne!(alice, bob);
    }
}
