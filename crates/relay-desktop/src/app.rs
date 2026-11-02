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

const INK: Color32 = Color32::from_rgb(29, 44, 56);
const MUTED: Color32 = Color32::from_rgb(98, 110, 117);
const PAPER: Color32 = Color32::from_rgb(246, 245, 240);
const LINE: Color32 = Color32::from_rgb(220, 224, 222);
const TEAL: Color32 = Color32::from_rgb(30, 113, 111);
const GREEN: Color32 = Color32::from_rgb(39, 118, 86);
const AMBER: Color32 = Color32::from_rgb(163, 107, 32);
const RED: Color32 = Color32::from_rgb(167, 60, 52);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Welcome,
    Send,
    Receive,
    History,
    Identity,
    Demo,
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
    /// Guided demo session (owned here; moved into worker and returned).
    demo_session: Option<Box<DemoSession>>,
    /// Last known public demo snapshot.
    demo_snapshot: Option<DemoSnapshot>,
}

impl RelayApp {
    pub fn new(context: &eframe::CreationContext<'_>, profile: Profile, home: PathBuf) -> Self {
        configure_style(&context.egui_ctx);
        let mut app = Self {
            profile,
            home,
            screen: Screen::Identity,
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
            demo_session: None,
            demo_snapshot: None,
        };
        app.start(&context.egui_ctx, Action::Refresh);
        app
    }

    fn busy(&self) -> bool {
        self.pending.is_some()
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

    fn poll(&mut self) {
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
                    if !self.loaded {
                        // First load: route to the right starting screen.
                        if self.identity.is_some() {
                            self.screen = Screen::Send;
                        } else if self.identity_missing {
                            self.screen = Screen::Welcome;
                        } else {
                            self.screen = Screen::Identity;
                        }
                        self.loaded = true;
                    }
                }
                match completed.payload {
                    Payload::None => {}
                    Payload::Measured(bytes) => self.selected_bytes = Some(bytes),
                    Payload::Sealed(details) => {
                        self.sealed_id = Some(details.record.parcel_id.clone());
                        self.selected_delivery =
                            Some((details.record.parcel_id.clone(), Direction::Outgoing));
                        self.sealed_inspection = Some(details.inspection);
                        self.sealed_file_bytes = Some(details.file_bytes);
                    }
                    Payload::Inspected(inspection) => self.inspection = Some(inspection),
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
        if self.screen != screen {
            self.password.zeroize();
            self.confirmation.zeroize();
            self.screen = screen;
        }
    }

    fn header(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("masthead")
            .frame(
                egui::Frame::new()
                    .fill(INK)
                    .inner_margin(egui::Margin::symmetric(24, 18)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("R / P")
                            .size(20.0)
                            .strong()
                            .color(Color32::from_rgb(158, 211, 198)),
                    );
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("RelayPost")
                                .size(24.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new("OFFLINE FILE HANDOFFS")
                                .size(10.0)
                                .color(Color32::from_rgb(177, 192, 200)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let identity = self
                            .identity
                            .as_ref()
                            .map(|card| card.name.as_str())
                            .unwrap_or(if self.identity_missing {
                                "Set up this device"
                            } else {
                                "Identity needs attention"
                            });
                        ui.label(RichText::new(identity).color(Color32::from_rgb(219, 231, 234)));
                    });
                });
            });
        egui::TopBottomPanel::top("navigation")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    for (screen, label) in [
                        (Screen::Send, "01  Send"),
                        (Screen::Receive, "02  Receive"),
                        (Screen::History, "03  History"),
                        (Screen::Identity, "04  Identity & Contacts"),
                        (Screen::Demo, "Demo"),
                    ] {
                        if ui
                            .selectable_label(self.screen == screen, RichText::new(label).strong())
                            .clicked()
                        {
                            self.switch_screen(screen);
                        }
                        ui.add_space(15.0);
                    }
                });
            });
    }

    fn footer(&self, context: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    if let Some(pending) = &self.pending {
                        ui.spinner();
                        ui.label(RichText::new(pending.label).color(TEAL));
                    } else {
                        ui.label(
                            RichText::new("LOCAL · NO NETWORK REQUIRED")
                                .size(10.0)
                                .color(MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Profile").small().color(MUTED))
                            .on_hover_text(self.home.display().to_string());
                        let keys = if cfg!(target_os = "macos") {
                            "⌘1–4 tabs  ·  ⌘R refresh"
                        } else {
                            "Ctrl+1–4 tabs  ·  Ctrl+R refresh"
                        };
                        ui.label(RichText::new(keys).small().color(MUTED));
                    });
                });
            });
    }

    fn messages(&mut self, ui: &mut Ui) {
        if let Some(error) = self.error.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(253, 238, 234))
                .inner_margin(egui::Margin::same(14))
                .corner_radius(6)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Operation could not finish")
                                    .strong()
                                    .color(RED),
                            );
                            ui.label(RichText::new(error).color(RED));
                        });
                        if ui.small_button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            ui.add_space(14.0);
        } else if !self.notice.is_empty() {
            egui::Frame::new()
                .fill(Color32::from_rgb(230, 242, 235))
                .inner_margin(egui::Margin::same(14))
                .corner_radius(6)
                .show(ui, |ui| {
                    ui.label(RichText::new(&self.notice).color(GREEN));
                });
            ui.add_space(14.0);
        }
    }

    fn send_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Prepare a handoff",
            "Seal a batch for one trusted device, then let a carrier take it there.",
        );
        if self.identity.is_none() {
            empty_state(
                ui,
                "Create this device's identity first",
                "Open Identity & Contacts to create an identity and exchange public cards.",
            );
            return;
        }
        surface(ui, |ui| {
            section_label(ui, "1  RECIPIENT");
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
                ui.label("No trusted contacts yet. Import a card and compare its complete fingerprint in Identity & Contacts.");
            } else {
                let selected = trusted
                    .iter()
                    .find(|contact| contact.card.fingerprint == self.recipient)
                    .map(|contact| contact.card.name.as_str())
                    .unwrap_or("Choose a trusted recipient");
                egui::ComboBox::from_id_salt("recipient")
                    .selected_text(selected)
                    .width(320.0)
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
                    .find(|contact| contact.card.fingerprint == self.recipient)
                {
                    fingerprint(ui, &contact.card.fingerprint);
                }
            }
        });
        ui.add_space(14.0);
        surface(ui, |ui| {
            section_label(ui, "2  FILES");
            ui.horizontal(|ui| {
                if ui.button("Add files…").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .set_title("Choose static regular files")
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
                if ui
                    .add_enabled(!self.inputs.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    self.inputs.clear();
                    self.selected_bytes = Some(0);
                    self.invalidate_sealed();
                }
                let size = self
                    .selected_bytes
                    .map(bytes)
                    .unwrap_or_else(|| "size not checked".to_owned());
                ui.label(
                    RichText::new(format!("{} files · {size}", self.inputs.len())).color(MUTED),
                );
            });
            if self.inputs.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Choose files that have finished writing.").color(MUTED));
            } else {
                let mut remove = None;
                egui::ScrollArea::vertical()
                    .id_salt("selected_files")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (index, input) in self.inputs.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(index);
                                }
                                ui.label(input.display().to_string());
                            });
                        }
                    });
                if let Some(index) = remove {
                    self.inputs.remove(index);
                    self.measure_inputs(ui.ctx());
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Up to 100 regular files · 64 MiB original data · encrypted only for the selected device").small().color(MUTED));
        });
        ui.add_space(14.0);
        surface(ui, |ui| {
            section_label(ui, "3  SEAL & EXPORT");
            password_field(ui, &mut self.password);
            let valid = !self.inputs.is_empty()
                && self.inputs.len() <= 100
                && self
                    .selected_bytes
                    .is_some_and(|size| size <= 64 * 1024 * 1024)
                && self
                    .contacts
                    .iter()
                    .any(|contact| contact.verified && contact.card.fingerprint == self.recipient)
                && !self.password.is_empty();
            if primary(ui, "Seal new parcel", valid).clicked() {
                let password = self.take_password();
                self.start(
                    ui.ctx(),
                    Action::Seal(self.inputs.clone(), self.recipient.clone(), password),
                );
            }
            ui.label(RichText::new("Sealing creates a new delivery. To retry an existing one, export it again from History.").small().color(MUTED));
            let sealed = self
                .sealed_id
                .as_ref()
                .and_then(|id| {
                    self.deliveries.iter().find(|record| {
                        record.parcel_id == *id && record.direction == Direction::Outgoing
                    })
                })
                .cloned();
            if let Some(record) = sealed {
                ui.add_space(16.0);
                ui.separator();
                draw_record(ui, &record);
                if let Some(file_bytes) = self.sealed_file_bytes {
                    ui.label(
                        RichText::new(format!("Exact parcel file size: {}", bytes(file_bytes)))
                            .strong(),
                    );
                }
                if let Some(inspection) = &self.sealed_inspection {
                    ui.label(format!(
                        "Encrypted content: {}",
                        bytes(inspection.ciphertext_bytes)
                    ));
                    ui.collapsing("Repair groups included in this parcel", |ui| {
                        for group in &inspection.groups {
                            ui.label(format!("Group {}: {} data + {} parity · repairs up to {} unavailable shards", group.index, group.data_shards, group.parity_shards, group.parity_shards));
                        }
                        ui.label(RichText::new("Repair budgets apply separately to each group.").small().color(MUTED));
                    });
                }
                if record.state != DeliveryState::Cancelled
                    && ui.button("Export parcel to carrier…").clicked()
                {
                    self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                }
            }
        });
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
            "Receive a parcel",
            "Check the signed envelope and repair budget before saving the batch.",
        );
        surface(ui, |ui| {
            section_label(ui, "1  PARCEL DIRECTORY");
            ui.horizontal(|ui| {
                let edited = ui
                    .add(
                        TextEdit::singleline(&mut self.parcel_input)
                            .hint_text("Path to the parcel directory")
                            .desired_width(600.0),
                    )
                    .changed();
                if edited {
                    self.inspection = None;
                    self.received = None;
                }
                if ui.button("Browse…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a RelayPost parcel directory")
                        .pick_folder()
                    {
                        self.parcel_input = path.display().to_string();
                        self.inspection = None;
                        self.received = None;
                    }
                }
            });
            ui.add_space(10.0);
            if primary(ui, "Inspect parcel", !self.parcel_input.trim().is_empty()).clicked() {
                self.inspection = None;
                self.received = None;
                self.start(ui.ctx(), Action::Inspect(PathBuf::from(&self.parcel_input)));
            }
        });
        if let Some(inspection) = self.inspection.clone() {
            ui.add_space(14.0);
            surface(ui, |ui| {
                section_label(ui, "2  AUTHENTICATED SHARD HEALTH");
                ui.horizontal(|ui| {
                    let (label, color) = if inspection.recoverable {
                        ("Sufficient shards", GREEN)
                    } else {
                        ("Repair budget exceeded", RED)
                    };
                    ui.label(RichText::new(label).strong().color(color));
                    ui.label(
                        RichText::new(format!("{} ciphertext", bytes(inspection.ciphertext_bytes)))
                            .color(MUTED),
                    );
                });
                ui.add_space(8.0);
                for group in &inspection.groups {
                    let unavailable = group.missing_shards.len();
                    let color = if !group.recoverable {
                        RED
                    } else if unavailable > 0 {
                        AMBER
                    } else {
                        GREEN
                    };
                    ui.label(
                        RichText::new(format!(
                            "Group {} · {} data + {} parity · {} valid · {} unavailable",
                            group.index,
                            group.data_shards,
                            group.parity_shards,
                            group.valid_shards,
                            unavailable
                        ))
                        .strong()
                        .color(color),
                    );
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
                                "Shard {index} · {} · {}",
                                if parity { "parity" } else { "data" },
                                if missing {
                                    "missing or hash mismatch"
                                } else {
                                    "digest verified"
                                }
                            ));
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "This group can repair at most {} missing or hash-bad shards.",
                            group.parity_shards
                        ))
                        .small()
                        .color(MUTED),
                    );
                    ui.add_space(9.0);
                }
                ui.label(RichText::new("Green: verified data   Blue: verified parity   Amber: unavailable / hash mismatch").small().color(MUTED));
                ui.collapsing("Authenticated envelope details", |ui| {
                    value(ui, "Parcel", &inspection.parcel_id);
                    value(ui, "Sender", &inspection.sender);
                    value(ui, "Recipient", &inspection.recipient);
                });
            });
            ui.add_space(14.0);
            surface(ui, |ui| {
                section_label(ui, "3  VERIFY & SAVE");
                password_field(ui, &mut self.password);
                if primary(
                    ui,
                    "Repair, verify & save",
                    inspection.recoverable && !self.password.is_empty(),
                )
                .clicked()
                {
                    let password = self.take_password();
                    self.start(
                        ui.ctx(),
                        Action::Receive(PathBuf::from(&self.parcel_input), password),
                    );
                }
                ui.label(RichText::new("A signed receipt is created only after the complete batch is verified and committed.").small().color(MUTED));
            });
        } else {
            ui.add_space(18.0);
            empty_state(ui, "Inspect before receiving", "The other device must be a pinned contact. Inspection verifies its signed envelope and checks actual shards without decrypting files.");
        }
        if let Some(result) = self.received.clone() {
            ui.add_space(14.0);
            surface(ui, |ui| {
                section_label(ui, "SAVED BATCH");
                ui.label(
                    RichText::new(format!(
                        "{} repaired shards · {}",
                        result.repaired_shards,
                        if result.duplicate {
                            "existing batch verified"
                        } else {
                            "new batch committed"
                        }
                    ))
                    .strong()
                    .color(GREEN),
                );
                draw_record(ui, &result.record);
                if result.record.receipt_path.is_some()
                    && ui.button("Export signed receipt to carrier…").clicked()
                {
                    self.choose_receipt_export(ui.ctx(), &result.record.parcel_id);
                }
                ui.label(RichText::new("Return the signed receipt to the sender so their delivery can be confirmed.").small().color(MUTED));
            });
        }
    }

    fn history_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Handoff history",
            "Local exports and verified recipient confirmations are distinct facts.",
        );
        ui.horizontal(|ui| {
            if ui.button("Refresh records").clicked() {
                self.start(ui.ctx(), Action::Refresh);
            }
            if ui.button("Import signed receipt…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose a signed receipt directory")
                    .pick_folder()
                {
                    self.start(ui.ctx(), Action::ImportReceipt(path));
                }
            }
        });
        ui.add_space(14.0);
        if self.deliveries.is_empty() {
            empty_state(ui, "No deliveries yet", "Seal a parcel in Send, or inspect and save a parcel in Receive. Each delivery will keep its own signed receipt state here.");
        } else {
            let records = self.deliveries.clone();
            surface(ui, |ui| {
                section_label(ui, "DELIVERIES");
                egui::ScrollArea::vertical()
                    .id_salt("delivery_list")
                    .max_height(225.0)
                    .show(ui, |ui| {
                        for record in &records {
                            ui.horizontal(|ui| {
                                let direction = match record.direction {
                                    Direction::Outgoing => "OUT",
                                    Direction::Incoming => "IN",
                                };
                                let label = format!(
                                    "{direction}  {}   ·   {} files / {}",
                                    short(&record.parcel_id),
                                    record.files,
                                    bytes(record.total_bytes)
                                );
                                if ui
                                    .selectable_label(
                                        self.selected_delivery.as_ref().is_some_and(
                                            |(id, direction)| {
                                                id == &record.parcel_id
                                                    && direction == &record.direction
                                            },
                                        ),
                                        label,
                                    )
                                    .clicked()
                                {
                                    self.selected_delivery =
                                        Some((record.parcel_id.clone(), record.direction.clone()));
                                }
                                state_badge(ui, &record.state);
                                if record.local_error.is_some() {
                                    ui.label(
                                        RichText::new("LOCAL COPY NEEDS ATTENTION")
                                            .small()
                                            .color(RED),
                                    );
                                }
                            });
                            ui.separator();
                        }
                    });
            });
            let selected = self
                .selected_delivery
                .as_ref()
                .and_then(|(id, direction)| {
                    self.deliveries
                        .iter()
                        .find(|record| &record.parcel_id == id && &record.direction == direction)
                })
                .cloned();
            if let Some(record) = selected {
                ui.add_space(14.0);
                surface(ui, |ui| {
                    section_label(ui, "DELIVERY DETAILS");
                    draw_record(ui, &record);
                    ui.horizontal(|ui| {
                        if record.direction == Direction::Outgoing
                            && record.state != DeliveryState::Cancelled
                        {
                            if ui.button("Export parcel again…").clicked() {
                                self.choose_parcel_export(ui.ctx(), &record.parcel_id);
                            }
                            if matches!(
                                record.state,
                                DeliveryState::Ready | DeliveryState::WaitingReceipt
                            ) && ui.button("Cancel future exports…").clicked()
                            {
                                self.cancel_id = Some(record.parcel_id.clone());
                            }
                        }
                        if record.direction == Direction::Incoming
                            && record.receipt_path.is_some()
                            && ui.button("Export signed receipt…").clicked()
                        {
                            self.choose_receipt_export(ui.ctx(), &record.parcel_id);
                        }
                    });
                });
            } else {
                ui.label(RichText::new("Select a delivery to see its full identifiers, saved paths, and available actions.").color(MUTED));
            }
        }
        ui.add_space(16.0);
        surface(ui, |ui| {
            ui.collapsing("Recover local records after an interrupted operation", |ui| {
                ui.label("Resume fully verified receive stages, check saved batches, and restore delivery records and receipts. Changed content is reported as a conflict.");
                password_field(ui, &mut self.password);
                if ui.add_enabled(!self.password.is_empty(), egui::Button::new("Verify & recover local records")).clicked() {
                    let password = self.take_password();
                    self.start(ui.ctx(), Action::Recover(password));
                }
                if let Some(summary) = &self.recovery {
                    ui.label(format!("{} recovered · {} pending · {} conflicts", summary.recovered, summary.pending, summary.conflicts.len()));
                    for conflict in &summary.conflicts { ui.label(RichText::new(conflict).color(RED)); }
                }
                ui.add_space(8.0);
                if ui.button("Discard interrupted receive work…").clicked() {
                    self.discard_incomplete = true;
                }
            });
        });
        ui.add_space(10.0);
        ui.label(RichText::new("A receipt records the recipient device's signed statement that this batch was saved. It does not imply a person read it or guarantee future storage.").small().color(MUTED));
    }

    fn identity_screen(&mut self, ui: &mut Ui) {
        page_title(ui, "Identity & Contacts", "Exchange public cards, compare the complete fingerprints, and pin trust on both devices.");
        surface(ui, |ui| {
            section_label(ui, "THIS DEVICE");
            if let Some(card) = self.identity.clone() {
                ui.label(RichText::new(&card.name).size(22.0).strong());
                fingerprint(ui, &card.fingerprint);
                ui.collapsing("Public key details", |ui| {
                    value(ui, "age recipient", &card.age_recipient);
                    value(ui, "Signing public key", &card.signing_public_key);
                    ui.label(format!("Card version {}", card.version));
                });
                if ui.button("Export public contact card…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Export public contact card")
                        .set_file_name("relaypost-card.json")
                        .add_filter("RelayPost public card", &["json"])
                        .save_file()
                    {
                        self.start(ui.ctx(), Action::ExportCard(path));
                    }
                }
                ui.label(RichText::new("This exports public keys only. Keep your password-protected profile directory safe; it contains the private identity.").small().color(MUTED));
            } else if !self.identity_missing {
                ui.label(RichText::new("Existing device identity needs attention").strong().color(RED));
                if let Some(error) = &self.identity_error {
                    ui.label(RichText::new(error).color(RED));
                }
                ui.label("RelayPost could not read this profile's existing identity. Restore an intact profile backup, or launch with --home pointing to a separate directory to create a new device identity.");
                if ui.button("Refresh local records").clicked() {
                    self.start(ui.ctx(), Action::Refresh);
                }
            } else {
                ui.label(
                    "Create a local identity. Its private keys are encrypted with your password.",
                );
                ui.label("Device name");
                ui.add(
                    TextEdit::singleline(&mut self.identity_name)
                        .hint_text("e.g. Field notebook")
                        .desired_width(400.0),
                );
                password_field(ui, &mut self.password);
                ui.label("Confirm password");
                ui.add(
                    TextEdit::singleline(&mut *self.confirmation)
                        .password(true)
                        .desired_width(400.0),
                );
                let valid = !self.identity_name.trim().is_empty()
                    && !self.password.is_empty()
                    && *self.password == *self.confirmation;
                if primary(ui, "Create device identity", valid).clicked() {
                    let password = self.take_password();
                    self.confirmation.zeroize();
                    self.start(
                        ui.ctx(),
                        Action::Initialize(self.identity_name.trim().to_owned(), password),
                    );
                }
                if !self.confirmation.is_empty() && *self.password != *self.confirmation {
                    ui.label(
                        RichText::new("Passwords do not match.")
                            .small()
                            .color(AMBER),
                    );
                }
            }
        });
        ui.add_space(14.0);
        surface(ui, |ui| {
            ui.horizontal(|ui| {
                section_label(ui, "CONTACTS");
                if ui.button("Import public card…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Import a public contact card")
                        .add_filter("Public card", &["json"])
                        .pick_file()
                    {
                        self.start(ui.ctx(), Action::ImportContact(path));
                    }
                }
            });
            if self.contacts.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "No contacts yet. Ask the other device to export its public card.",
                    )
                    .color(MUTED),
                );
            }
            for contact in self.contacts.clone() {
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&contact.card.name).size(17.0).strong());
                    ui.label(
                        RichText::new(if contact.verified {
                            "FINGERPRINT PINNED"
                        } else {
                            "UNVERIFIED"
                        })
                        .small()
                        .color(if contact.verified {
                            GREEN
                        } else {
                            AMBER
                        }),
                    );
                });
                fingerprint(ui, &contact.card.fingerprint);
                if !contact.verified && ui.button("Verify complete fingerprint…").clicked() {
                    self.trust_contact = Some(contact);
                    self.trust_fingerprint.clear();
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("A valid card signature binds the public keys. Compare the full fingerprint through a trusted channel before pinning it. Each device must pin the other.").small().color(MUTED));
        });
        ui.add_space(12.0);
        ui.collapsing("Profile location", |ui| {
            value(ui, "Local app data", &self.home.display().to_string());
            ui.label(RichText::new("The CLI uses this same profile by default. Launch with --home to use an isolated profile.").small().color(MUTED));
        });
    }

    fn welcome_screen(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Welcome to RelayPost",
            "Carry encrypted files offline. Verify their arrival.",
        );
        surface(ui, |ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "RelayPost encrypts files for a trusted recipient, carries the parcel \
                     offline via USB drive or any shared directory, and proves the recipient \
                     saved it via a returned signed receipt. No accounts, no cloud, no \
                     network connection required.",
                )
                .size(15.0),
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if primary(ui, "Try guided demo", !self.busy()).clicked() {
                    let ctx = ui.ctx().clone();
                    self.start_demo(&ctx);
                }
                ui.add_space(8.0);
                if ui.button("Set up my device").clicked() {
                    self.switch_screen(Screen::Identity);
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "The guided demo runs two synthetic profiles on this machine — \
                     no real keys or files are touched.",
                )
                .small()
                .color(MUTED),
            );
        });
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
            "Guided Demo",
            "Local one-machine directory simulation — no real keys, no user files, no physical USB drive.",
        );

        // "Run new guided demo" always available (not gated on busy — primary handles that).
        {
            let ctx = ui.ctx().clone();
            if primary(ui, "Run new guided demo", !self.busy()).clicked() {
                self.start_demo(&ctx);
            }
        }
        ui.label(
            RichText::new(
                "Each run creates a fresh private temp directory. Old demo folders are preserved. \
                 Two synthetic profiles (Alice and Bob) exchange two generated files (20 MiB + 1 MiB) using \
                 the real production cryptographic and Reed–Solomon engine.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(12.0);

        // Clone snapshot to avoid borrow conflicts during draw.
        let snap = match self.demo_snapshot.clone() {
            Some(s) => s,
            None => {
                surface(ui, |ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new("What will happen").strong());
                    ui.add_space(4.0);
                    ui.label("1. Two synthetic identities (Alice and Bob) are created in a private temp directory.");
                    ui.label("2. Two synthetic files (20 MiB + 1 MiB) are generated, sealed into a parcel with Reed–Solomon parity shards, and exported to a simulated carrier directory.");
                    ui.label("3. You advance through Damaged → Repair and receive → Carry back signed receipt, watching the engine refuse an over-budget receive and then repair the remaining erasures.");
                    ui.label("4. Alice's delivery state reaches Confirmed. A JSON evidence report is exportable.");
                    ui.add_space(8.0);
                });
                return;
            }
        };

        ui.label(RichText::new(format!("{} synthetic source files", snap.source_sha256.len())).color(MUTED));
        if !self.busy() && self.demo_session.is_none() {
            ui.label(RichText::new("This demo session stopped. The last successful phase is shown below. Run a new guided demo to continue with a fresh directory.").color(RED));
            ui.add_space(8.0);
        }

        // Phase badge + purpose description.
        ui.horizontal(|ui| {
            let (badge, color, purpose) = match snap.phase {
                DemoPhase::Prepared => (
                    "PREPARED",
                    GREEN,
                    "Parcel sealed and exported to carrier. Inspect initial shard health before any damage.",
                ),
                DemoPhase::Damaged => (
                    "DAMAGED",
                    AMBER,
                    "Carrier copy damaged: 2 data shards deleted, 2 parity shards XOR-corrupted. \
                     A 5-erasure receive was refused (over budget). One shard was restored, \
                     leaving the group within repair budget.",
                ),
                DemoPhase::Received => (
                    "RECEIVED",
                    GREEN,
                    "Bob repaired the erasures and received the batch. SHA-256 hashes verified. \
                     Signed receipt ready — it will be exported and carried back to Alice next.",
                ),
                DemoPhase::Confirmed => (
                    "CONFIRMED",
                    GREEN,
                    "Alice verified Bob's signed receipt. Delivery is Confirmed. \
                     Export the JSON evidence report to keep a record.",
                ),
            };
            ui.label(RichText::new(badge).size(11.0).strong().color(color));
            ui.label(RichText::new(purpose).color(MUTED));
        });
        ui.add_space(8.0);

        // Paths.
        surface(ui, |ui| {
            ui.label(RichText::new("Demo directory layout").strong());
            value(ui, "Root", &snap.root.display().to_string());
            if let Some(p) = &snap.carrier_parcel_path {
                value(ui, "Carrier parcel", &p.display().to_string());
            }
            if let Some(p) = &snap.received_path {
                value(ui, "Received batch", &p.display().to_string());
            }
            if let Some(p) = &snap.receipt_path {
                value(ui, "Signed receipt", &p.display().to_string());
            }
            if let Some(p) = &snap.report_path {
                value(ui, "JSON evidence report", &p.display().to_string());
            }
        });
        ui.add_space(8.0);

        // Sender delivery state.
        if let Some(state) = &snap.sender_state {
            surface(ui, |ui| {
                ui.label(RichText::new("Alice's sender state").strong());
                state_badge(ui, state);
                ui.label(state_description(state));
            });
            ui.add_space(8.0);
        }

        // Initial inspection (always shown once available).
        if let Some(insp) = &snap.initial_inspection {
            surface(ui, |ui| {
                ui.label(RichText::new("Initial carrier inspection").strong());
                ui.label(
                    RichText::new("Signed envelope verified. Shard health before any damage:")
                        .color(MUTED),
                );
                for g in &insp.groups {
                    let missing_data = g.missing_shards.iter().filter(|&&index| index < g.data_shards).count();
                    let missing_parity = g.missing_shards.iter().filter(|&&index| index >= g.data_shards).count();
                    let bad = g.missing_shards.len();
                    let kind = if g.index + 1 < insp.groups.len() { "full" } else { "tail" };
                    let status = if g.recoverable { "✓ recoverable" } else { "✗ not recoverable" };
                    ui.label(format!(
                        "Group {} ({kind}): {}/{} data, {}/{} parity — {} missing/bad — {status}",
                        g.index,
                        g.data_shards - missing_data,
                        g.data_shards,
                        g.parity_shards - missing_parity,
                        g.parity_shards,
                        bad,
                    ));
                }
                ui.label(format!(
                    "Overall: {} — {} bytes ciphertext",
                    if insp.recoverable { "all groups recoverable" } else { "some groups not recoverable" },
                    insp.ciphertext_bytes
                ));
            });
            ui.add_space(8.0);
        }

        // Damaged inspection + 5-erasure evidence.
        if let Some(insp) = &snap.inspection_after_damage {
            surface(ui, |ui| {
                ui.label(RichText::new("Inspection after damage").strong());
                ui.label(
                    RichText::new(
                        "2 data shards missing (deleted), 2 parity shards hash-bad (XOR corrupted). \
                         4 erasures — within repair budget.",
                    )
                    .color(AMBER),
                );
                if !snap.damaged_shard_indices.is_empty() {
                    ui.label(format!("Damaged shard indices: {:?}", snap.damaged_shard_indices));
                }
                for g in &insp.groups {
                    let missing_data = g.missing_shards.iter().filter(|&&index| index < g.data_shards).count();
                    let missing_parity = g.missing_shards.iter().filter(|&&index| index >= g.data_shards).count();
                    let bad = g.missing_shards.len();
                    let status = if g.recoverable { "✓ recoverable" } else { "✗ NOT recoverable" };
                    ui.label(format!(
                        "Group {}: {}/{} data, {}/{} parity — {} erasures — {status}",
                        g.index, g.data_shards - missing_data, g.data_shards,
                        g.parity_shards - missing_parity, g.parity_shards, bad,
                    ));
                }
            });
            ui.add_space(4.0);
        }

        // 5-erasure refusal evidence.
        if let Some(insp5) = &snap.five_erasure_refusal_evidence {
            surface(ui, |ui| {
                ui.label(
                    RichText::new("5-erasure receive refused (over budget)")
                        .strong()
                        .color(RED),
                );
                if let Some(err) = &snap.five_erasure_receive_error {
                    ui.label(RichText::new(err).color(RED));
                }
                ui.label(
                    RichText::new(
                        "Temporarily adding a 5th erasure (shard 2) caused a refusal. \
                         Shard 2 was then restored, bringing erasures back to 4.",
                    )
                    .small()
                    .color(MUTED),
                );
                for g in &insp5.groups {
                    let bad = g.missing_shards.len();
                    let status = if g.recoverable { "within budget" } else { "OVER BUDGET — refused" };
                    ui.label(format!("Group {}: {} erasures — {status}", g.index, bad));
                }
            });
            ui.add_space(4.0);
        }

        // Repaired shard count (after receive).
        if let Some(repaired) = snap.repaired_shards {
            surface(ui, |ui| {
                ui.label(RichText::new("Reed–Solomon repair").strong());
                ui.label(format!("{repaired} shards reconstructed from parity."));
            });
            ui.add_space(4.0);
        }

        // File SHA-256 comparison.
        if !snap.source_sha256.is_empty() {
            surface(ui, |ui| {
                ui.label(RichText::new("File integrity — SHA-256").strong());
                let have_received = !snap.received_sha256.is_empty();
                for (name, src_hash) in &snap.source_sha256 {
                    ui.add_space(4.0);
                    ui.label(RichText::new(name).strong());
                    value(ui, "Source   SHA-256", src_hash);
                    if have_received {
                        if let Some(rcv_hash) = snap.received_sha256.get(name) {
                            value(ui, "Received SHA-256", rcv_hash);
                            let matches = src_hash == rcv_hash;
                            let (mark, color) = if matches {
                                ("✓ MATCH — cryptographic integrity verified", GREEN)
                            } else {
                                ("✗ MISMATCH", RED)
                            };
                            ui.label(RichText::new(mark).strong().color(color));
                        }
                    }
                }
                if !have_received {
                    ui.label(
                        RichText::new("Received hashes available after Bob receives the parcel.")
                            .small()
                            .color(MUTED),
                    );
                }
            });
            ui.add_space(8.0);
        }

        // Action button + export report.
        let ctx = ui.ctx().clone();
        let can_advance = !self.busy() && self.demo_session.is_some();

        match snap.phase {
            DemoPhase::Prepared => {
                ui.add_space(4.0);
                let btn = ui.add_enabled(can_advance, egui::Button::new(
                    RichText::new("Damage carrier copy →").strong(),
                ));
                if btn.clicked() {
                    if let Some(session) = self.demo_session.take() {
                        self.start(&ctx, Action::DemoAdvance(session));
                    }
                }
                ui.label(
                    RichText::new(
                        "Will delete 2 data shards, XOR-corrupt 2 parity shards, attempt a \
                         5-erasure receive (expecting refusal), then restore shard 2.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
            DemoPhase::Damaged => {
                ui.add_space(4.0);
                let btn = ui.add_enabled(can_advance, egui::Button::new(
                    RichText::new("Repair and receive →").strong(),
                ));
                if btn.clicked() {
                    if let Some(session) = self.demo_session.take() {
                        self.start(&ctx, Action::DemoAdvance(session));
                    }
                }
                ui.label(
                    RichText::new(
                        "Bob's profile will repair the 4 erasures, decrypt and verify the batch, \
                         then create a signed receipt ready for the return trip.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
            DemoPhase::Received => {
                ui.add_space(4.0);
                let btn = ui.add_enabled(can_advance, egui::Button::new(
                    RichText::new("Carry back signed receipt →").strong(),
                ));
                if btn.clicked() {
                    if let Some(session) = self.demo_session.take() {
                        self.start(&ctx, Action::DemoAdvance(session));
                    }
                }
                ui.label(
                    RichText::new(
                        "Bob's signed receipt is exported to the carrier, then Alice imports it. Delivery reaches Confirmed.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
            DemoPhase::Confirmed => {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "Demo complete. All phases verified with the real cryptographic engine.",
                    )
                    .strong()
                    .color(GREEN),
                );
                // Export report button — only when report_path populated and not busy.
                if let Some(report_src) = snap.report_path.clone() {
                    ui.add_space(4.0);
                    let btn = ui.add_enabled(can_advance, egui::Button::new(
                        RichText::new("Export JSON evidence report…").strong(),
                    ));
                    if btn.clicked() {
                        if let Some(target) = rfd::FileDialog::new()
                            .set_file_name("relaypost-demo-report.json")
                            .save_file()
                        {
                            self.start(&ctx, Action::DemoCopyReport(report_src, target));
                        }
                    }
                    ui.label(
                        RichText::new("Save a copy of the JSON evidence report to a location of your choice.")
                            .small()
                            .color(MUTED),
                    );
                }
            }
        }
    }

    fn choose_parcel_export(&mut self, context: &egui::Context, id: &str) {
        if let Some(carrier) = rfd::FileDialog::new()
            .set_title("Choose carrier directory for the parcel")
            .pick_folder()
        {
            self.start(context, Action::ExportParcel(id.to_owned(), carrier));
        }
    }

    fn choose_receipt_export(&mut self, context: &egui::Context, id: &str) {
        if let Some(carrier) = rfd::FileDialog::new()
            .set_title("Choose carrier directory for the signed receipt")
            .pick_folder()
        {
            self.start(context, Action::ExportReceipt(id.to_owned(), carrier));
        }
    }

    fn dialogs(&mut self, context: &egui::Context) {
        if let Some(contact) = self.trust_contact.clone() {
            let mut close = false;
            egui::Window::new("Pin a verified contact")
                .collapsible(false)
                .resizable(false)
                .default_width(570.0)
                .show(context, |ui| {
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        ui.label(format!(
                            "Compare {}'s full fingerprint in person or through a trusted channel.",
                            contact.card.name
                        ));
                        fingerprint(ui, &contact.card.fingerprint);
                        ui.add_space(10.0);
                        ui.label("Paste the complete fingerprint you independently confirmed");
                        ui.add(
                            TextEdit::singleline(&mut self.trust_fingerprint)
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if primary(
                                ui,
                                "Pin verified fingerprint",
                                self.trust_fingerprint.trim() == contact.card.fingerprint,
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
            egui::Window::new("Cancel future exports?").collapsible(false).resizable(false).default_width(460.0).show(context, |ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    ui.label("This stops local re-export of this parcel. It cannot recall copies already on a carrier and does not delete the original source files.");
                    value(ui, "Parcel", &id);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Cancel future exports").color(RED)).clicked() {
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
                        ui.horizontal(|ui| {
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
        for (key, screen) in [
            (egui::Key::Num1, Screen::Send),
            (egui::Key::Num2, Screen::Receive),
            (egui::Key::Num3, Screen::History),
            (egui::Key::Num4, Screen::Identity),
        ] {
            if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, key)) {
                self.switch_screen(screen);
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::R))
            && !self.busy()
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
        self.poll();
        self.shortcuts(context);
        self.header(context);
        self.footer(context);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                egui::ScrollArea::vertical().id_salt("page").show(ui, |ui| {
                    self.messages(ui);
                    ui.add_enabled_ui(!self.busy(), |ui| match self.screen {
                        Screen::Welcome => self.welcome_screen(ui),
                        Screen::Send => self.send_screen(ui),
                        Screen::Receive => self.receive_screen(ui),
                        Screen::History => self.history_screen(ui),
                        Screen::Identity => self.identity_screen(ui),
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

fn configure_style(context: &egui::Context) {
    context.set_visuals(egui::Visuals::light());
    let mut style = (*context.style()).clone();
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = Color32::from_rgb(208, 232, 224);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEAL);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(212, 233, 225);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(229, 240, 233);
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(28.0));
    context.set_style(style);
}

fn page_title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.heading(title);
    ui.label(RichText::new(subtitle).color(MUTED));
    ui.add_space(18.0);
}

fn section_label(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).size(11.0).strong().color(MUTED));
    ui.add_space(4.0);
}

fn surface(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(egui::Stroke::new(1.0_f32, LINE))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

fn primary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(TEAL),
    )
}

fn empty_state(ui: &mut Ui, title: &str, body: &str) {
    surface(ui, |ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(title).size(19.0).strong());
        ui.label(RichText::new(body).color(MUTED));
        ui.add_space(12.0);
    });
}

fn password_field(ui: &mut Ui, password: &mut Zeroizing<String>) {
    ui.add_space(5.0);
    ui.label("Identity password · for this operation only");
    ui.add(
        TextEdit::singleline(&mut **password)
            .password(true)
            .desired_width(400.0)
            .hint_text("Password"),
    );
    ui.add_space(5.0);
}

fn fingerprint(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new("COMPLETE FINGERPRINT")
            .size(10.0)
            .color(MUTED),
    );
    value(ui, "", text);
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
        ui.label(RichText::new(format!("Local copy needs attention: {error}")).color(RED));
    }
    ui.add_space(6.0);
    ui.label(format!(
        "{} files · {} original data",
        record.files,
        bytes(record.total_bytes)
    ));
    value(ui, "Parcel", &record.parcel_id);
    ui.collapsing("Identities & local times", |ui| {
        value(ui, "Sender", &record.sender);
        value(ui, "Recipient", &record.recipient);
        ui.label(format!("Created locally: {}", record.created_at));
        ui.label(format!("Last local update: {}", record.updated_at));
    });
    if let Some(path) = &record.parcel_path {
        value(ui, "Local sealed parcel", &path.display().to_string());
    }
    if let Some(path) = &record.received_path {
        value(ui, "Saved batch", &path.display().to_string());
    }
    if let Some(path) = &record.receipt_path {
        value(ui, "Signed receipt", &path.display().to_string());
    }
}

fn state_badge(ui: &mut Ui, state: &DeliveryState) {
    let (label, color) = match state {
        DeliveryState::Ready => ("READY TO EXPORT", TEAL),
        DeliveryState::WaitingReceipt => ("AWAITING SIGNED RECEIPT", AMBER),
        DeliveryState::SavedReceiptReady => ("SAVED · RECEIPT READY", GREEN),
        DeliveryState::Confirmed => ("RECIPIENT CONFIRMED SAVED", GREEN),
        DeliveryState::Failed => ("NEEDS ATTENTION", RED),
        DeliveryState::Cancelled => ("LOCAL EXPORTS CANCELLED", MUTED),
    };
    ui.label(RichText::new(label).size(11.0).strong().color(color));
}

fn state_description(state: &DeliveryState) -> &'static str {
    match state {
        DeliveryState::Ready => "The immutable parcel is sealed locally and ready for a carrier.",
        DeliveryState::WaitingReceipt => "The parcel was exported locally. A verified recipient confirmation has not returned.",
        DeliveryState::SavedReceiptReady => "This device saved the verified batch. Carry its signed receipt back to the sender.",
        DeliveryState::Confirmed => "The expected recipient's matching signed receipt has been verified.",
        DeliveryState::Failed => "The local delivery needs attention. Review the error and verify local content before retrying.",
        DeliveryState::Cancelled => "Further local exports are stopped. Previously exported copies cannot be recalled.",
    }
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
