#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use eframe::egui::{self, Color32, RichText, Vec2};
use ferrite_core::i18n::{self, Language, tf, tr};
use ferrite_core::{
    builder::{self, Build},
    capture,
    compatibility::{self, Analysis, Status},
    disks::{self, Disk},
    efi::{self, Audit},
    elevation,
    planner::{self, Options, Plan},
    recovery::{self, Recovery},
    report::Report,
    workspace,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const BG: Color32 = Color32::from_rgb(31, 35, 42);
const PANEL: Color32 = Color32::from_rgb(36, 40, 48);
const CARD: Color32 = Color32::from_rgb(44, 49, 58);
const BORDER: Color32 = Color32::from_rgb(54, 60, 71);
const TEXT: Color32 = Color32::from_rgb(212, 216, 225);
const MUTED: Color32 = Color32::from_rgb(133, 143, 159);
const ACCENT: Color32 = Color32::from_rgb(138, 176, 243);
const WARNING: Color32 = Color32::from_rgb(242, 193, 112);
const ERROR: Color32 = Color32::from_rgb(243, 132, 137);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Overview,
    Hardware,
    Efi,
    Usb,
    Log,
    Settings,
    Guide,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => tr("nav.overview"),
            Self::Hardware => tr("nav.hardware"),
            Self::Efi => "OpenCore EFI",
            Self::Usb => tr("nav.usb"),
            Self::Log => tr("nav.log"),
            Self::Settings => tr("nav.settings"),
            Self::Guide => tr("nav.guide"),
        }
    }
}
enum Message {
    Report(Box<Report>, String),
    Disks(Vec<Disk>),
    Audit(Audit),
    Saved(String),
    Failure(String),
    Progress(String, u64, u64),
    Build(Box<Build>),
    Recovery(Recovery),
    Written(String),
}
struct App {
    page: Page,
    target: u8,
    report: Option<Report>,
    analysis: Option<Analysis>,
    disks: Vec<Disk>,
    audit: Option<Audit>,
    root: PathBuf,
    worker: Option<Receiver<Message>>,
    log: Vec<String>,
    search: String,
    only_problems: bool,
    notice: String,
    source: String,
    acpi_folder: Option<PathBuf>,
    build_result: Option<Box<Build>>,
    recovery_result: Option<Recovery>,
    options: Options,
    iasl: String,
    selected_disk: Option<String>,
    confirmation: String,
    progress: Option<f32>,
    language: Language,
    bios_checks: [bool; 8],
    install_checks: [bool; 6],
    tabs: Vec<Page>,
    palette_open: bool,
    palette_query: String,
    palette_index: usize,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.22);
        style.visuals.selection.stroke = egui::Stroke::new(1_f32, ACCENT);
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.button_padding = Vec2::new(10., 5.);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(3);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1_f32, BORDER);
        style.visuals.widgets.hovered.bg_fill = CARD;
        style.visuals.widgets.active.bg_fill = BORDER;
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(3);
        style.visuals.window_stroke = egui::Stroke::new(1_f32, BORDER);
        style.visuals.window_corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.inactive.bg_fill = CARD;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.));
        cc.egui_ctx.set_style(style);
        let language = preferences::load();
        #[cfg(feature = "screenshot")]
        let language = std::env::var("FERRITE_PREVIEW_LANGUAGE")
            .map(|s| Language::from_locale(&s))
            .unwrap_or(language);
        i18n::set_language(language);
        let mut app = Self {
            page: Page::Overview,
            target: 24,
            report: None,
            analysis: None,
            disks: vec![],
            audit: None,
            root: workspace::default_workspace(),
            worker: None,
            log: vec![format!(
                "Ferrite {} · {} · {}",
                ferrite_core::VERSION,
                std::env::consts::OS,
                std::env::consts::ARCH
            )],
            search: String::new(),
            only_problems: false,
            notice: String::new(),
            source: String::new(),
            acpi_folder: None,
            build_result: None,
            recovery_result: None,
            options: Options::default(),
            iasl: ferrite_core::platform::default_iasl(),
            selected_disk: None,
            confirmation: String::new(),
            progress: None,
            language,
            bios_checks: [false; 8],
            install_checks: [false; 6],
            tabs: vec![Page::Overview],
            palette_open: false,
            palette_query: String::new(),
            palette_index: 0,
        };
        if let Some(file) = std::env::args_os().nth(1) {
            let path = PathBuf::from(file);
            app.import(path);
        }
        #[cfg(feature = "screenshot")]
        if std::env::var("FERRITE_PREVIEW_PAGE").as_deref() == Ok("hardware") {
            app.page = Page::Hardware;
        }
        #[cfg(feature = "screenshot")]
        if std::env::var("FERRITE_PREVIEW_PAGE").as_deref() == Ok("efi") {
            app.page = Page::Efi;
        }
        #[cfg(feature = "screenshot")]
        if std::env::var("FERRITE_PREVIEW_PAGE").as_deref() == Ok("guide") {
            app.page = Page::Guide;
        }
        #[cfg(feature = "screenshot")]
        if std::env::var("FERRITE_PREVIEW_PAGE").as_deref() == Ok("commands") {
            app.palette_open = true;
        }
        app
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> anyhow::Result<Message> + Send + 'static,
    ) {
        self.start_stream(ctx, move |_| job());
    }
    fn start_stream(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce(mpsc::Sender<Message>) -> anyhow::Result<Message> + Send + 'static,
    ) {
        if self.worker.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        self.worker = Some(receive);
        self.notice = tr("task.running").into();
        self.progress = None;
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(send.clone())));
            let message = match result {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => Message::Failure(format!("{e:#}")),
                Err(_) => Message::Failure(tr("task.stopped").into()),
            };
            let _ = send.send(message);
            ctx.request_repaint();
        });
    }
    fn reset_workflow(&mut self) {
        self.bios_checks = [false; 8];
        self.install_checks = [false; 6];
        self.build_result = None;
        self.recovery_result = None;
        self.options.target = self.target;
        self.acpi_folder = std::path::Path::new(&self.source)
            .parent()
            .map(|p| p.join("ACPI"))
            .filter(|p| p.is_dir());
    }
    fn import(&mut self, path: PathBuf) {
        match Report::load(&path) {
            Ok(report) => {
                self.source = path.display().to_string();
                self.reset_workflow();
                self.analysis = Some(compatibility::analyze(&report, self.target));
                self.report = Some(report);
                self.notice = tr("report.imported").into();
                self.log
                    .push(tf("report.import", &[("0", format!("{}", path.display()))]));
            }
            Err(e) => {
                self.notice = tr("task.failed").into();
                self.log
                    .push(tf("report.error", &[("e", format!("{:#}", e))]));
            }
        }
    }
    fn choose_report(&mut self, ctx: &egui::Context) {
        self.start(ctx, || {
            let path = rfd::FileDialog::new()
                .add_filter(tr("dialog.report"), &["json"])
                .pick_file();
            match path {
                Some(path) => Ok(Message::Report(
                    Box::new(Report::load(&path)?),
                    path.display().to_string(),
                )),
                None => Ok(Message::Saved(tr("task.cancelled").into())),
            }
        });
    }
    fn poll(&mut self) {
        for _ in 0..512 {
            let Some(result) = self.worker.as_ref().map(|w| w.try_recv()) else {
                break;
            };
            match result {
                Ok(Message::Progress(name, done, total)) => {
                    self.notice = tr("task.running").into();
                    let _ = name;
                    self.progress = if total > 0 {
                        Some(done as f32 / total as f32)
                    } else {
                        None
                    };
                }
                Ok(message) => {
                    self.worker = None;
                    self.progress = None;
                    match message {
                        Message::Report(report, source) => {
                            self.source = source;
                            self.reset_workflow();
                            self.analysis = Some(compatibility::analyze(&report, self.target));
                            self.report = Some(*report);
                            self.notice = tr("hardware.checked").into();
                        }
                        Message::Disks(disks) => {
                            self.selected_disk = None;
                            self.confirmation.clear();
                            self.disks = disks;
                            self.notice =
                                tf("usb.found", &[("0", format!("{}", self.disks.len()))]);
                        }
                        Message::Audit(a) => {
                            self.notice = tf("efi.issues", &[("0", format!("{}", a.issues.len()))]);
                            self.audit = Some(a);
                        }
                        Message::Build(build) => {
                            self.notice = tr("efi.built").into();
                            self.build_result = Some(build);
                            self.recovery_result = None;
                        }
                        Message::Recovery(r) => {
                            self.notice = tr("recovery.ready").into();
                            self.recovery_result = Some(r);
                            self.page = Page::Usb;
                        }
                        Message::Written(text) | Message::Saved(text) => self.notice = text,
                        Message::Failure(text) => {
                            self.log.push(text);
                            self.notice = tr("task.failed").into();
                        }
                        Message::Progress(_, _, _) => unreachable!(),
                    }
                    self.log.push(self.notice.clone());
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.notice = tr("task.no_result").into();
                    self.log.push(self.notice.clone());
                    break;
                }
            }
        }
    }
    fn scan(&mut self, ctx: &egui::Context) {
        let root = self.root.clone();
        self.start(ctx, move || {
            let captured = capture::snapshot(&root)?;
            Ok(Message::Report(
                Box::new(Report::load(&captured.report)?),
                captured.report.display().to_string(),
            ))
        });
    }
    fn save_analysis(&mut self, ctx: &egui::Context) {
        if let Some(a) = self.analysis.clone() {
            let root = self.root.clone();
            self.start(ctx, move || {
                let p = workspace::save_new(&root, "compatibility.json", &a)?;
                Ok(Message::Saved(tf(
                    "report.saved",
                    &[("0", format!("{}", p.display()))],
                )))
            })
        }
    }
    fn version(&mut self, ui: &mut egui::Ui) {
        if self.worker.is_some() {
            ui.label(compatibility::version_name(self.target));
            return;
        }
        let previous = self.target;
        egui::ComboBox::from_id_salt("macos")
            .selected_text(format!(
                "macOS {}",
                compatibility::version_name(self.target)
            ))
            .show_ui(ui, |ui| {
                for (major, name) in compatibility::VERSIONS.into_iter().rev() {
                    ui.selectable_value(&mut self.target, major, name);
                }
            });
        if previous != self.target
            && let Some(report) = &self.report
        {
            self.analysis = Some(compatibility::analyze(report, self.target));
            self.build_result = None;
            self.recovery_result = None;
        }
    }
    fn overview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let wide = ui.available_width();
        let busy = self.worker.is_some();
        if cfg!(target_os = "macos") {
            ui.label(RichText::new(tr("mac.prepare_host")).color(MUTED));
            ui.add_space(12.);
        }
        egui::Frame::new()
            .fill(BG)
            .corner_radius(0)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_min_width(wide);
                ui.label(
                    RichText::new(tr("overview.eyebrow"))
                        .size(12.)
                        .color(ACCENT),
                );
                ui.add_space(8.);
                ui.label(RichText::new(tr("overview.title")).size(20.).strong());
                ui.label(RichText::new(tr("overview.subtitle")).color(MUTED));
                ui.add_space(14.);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(RichText::new(tr("action.scan_pc")).color(BG))
                                .fill(ACCENT),
                        )
                        .clicked()
                    {
                        self.scan(ctx);
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new(tr("action.import")))
                        .clicked()
                    {
                        self.choose_report(ctx);
                    }
                });
            });
        ui.add_space(18.);
        ui.columns(3, |columns| {
            let summary = self.analysis.as_ref();
            stat(
                &mut columns[0],
                tr("summary.hardware"),
                &summary
                    .map(|a| a.devices.len().to_string())
                    .unwrap_or_else(|| "—".into()),
                tr("summary.devices"),
            );
            stat(
                &mut columns[1],
                tr("summary.compatibility"),
                summary
                    .map(|a| {
                        if a.ok {
                            tr("summary.supported")
                        } else {
                            tr("summary.limits")
                        }
                    })
                    .unwrap_or(tr("summary.unchecked")),
                tr("summary.description"),
            );
            stat(
                &mut columns[2],
                tr("summary.platform"),
                ferrite_core::platform::host_name(),
                tr("summary.platform_description"),
            );
        });
        ui.add_space(22.);
        ui.label(RichText::new(tr("overview.steps")).size(21.).strong());
        for (number, title, detail, page) in [
            (
                "01",
                tr("nav.hardware"),
                tr("overview.hardware_detail"),
                Page::Hardware,
            ),
            ("02", "OpenCore EFI", tr("overview.efi_detail"), Page::Efi),
            (
                "03",
                tr("overview.usb_title"),
                tr("overview.usb_detail"),
                Page::Usb,
            ),
        ] {
            egui::Frame::new()
                .fill(PANEL)
                .corner_radius(3)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_min_width(wide - 24.);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(number).size(20.).color(ACCENT));
                        ui.vertical(|ui| {
                            ui.strong(title);
                            ui.label(RichText::new(detail).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr("action.open")).clicked() {
                                self.page = page;
                            }
                        });
                    });
                });
        }
        if let Some(a) = &self.analysis {
            ui.add_space(14.);
            if !a.blockers.is_empty() {
                ui.label(RichText::new(tr("hardware.blocked")).color(ERROR));
            }
        }
    }
    fn hardware(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            self.version(ui);
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new(tr("action.import_report")),
                )
                .clicked()
            {
                self.choose_report(ctx);
            }
            if ui
                .add_enabled(self.worker.is_none(), egui::Button::new(tr("action.scan")))
                .clicked()
            {
                self.scan(ctx);
            }
            if ui
                .add_enabled(
                    self.analysis.is_some() && self.worker.is_none(),
                    egui::Button::new(tr("action.export_json")),
                )
                .clicked()
            {
                self.save_analysis(ctx);
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr("hardware.search"))
                    .desired_width(360.),
            );
            ui.checkbox(&mut self.only_problems, tr("hardware.only_limits"));
        });
        ui.add_space(8.);
        if let Some(a) = &self.analysis {
            if a.ok {
                ui.label(RichText::new(tr("hardware.compatible")).color(ACCENT));
            }
            if !a.blockers.is_empty() {
                ui.label(RichText::new(tr("hardware.blocked")).color(ERROR));
                guide_ui::diagnostic(ui, a.blockers.iter().map(String::as_str));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("hardware.native_versions"));
                for v in &a.native_versions {
                    ui.label(RichText::new(compatibility::version_name(*v)).color(ACCENT));
                }
            });
            let query = self.search.to_lowercase();
            for d in &a.devices {
                if self.only_problems && d.status == Status::Supported {
                    continue;
                }
                if !format!("{} {} {}", d.name, d.id, d.section)
                    .to_lowercase()
                    .contains(&query)
                {
                    continue;
                }
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(3)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width() - 24.);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(guide_ui::section_label(&d.section))
                                    .size(12.)
                                    .color(MUTED),
                            );
                            ui.label(
                                RichText::new(guide_ui::status_label(d.status))
                                    .color(status_color(d.status)),
                            );
                        });
                        ui.strong(&d.name);
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(&d.id);
                            if let Some(r) = d.native {
                                ui.label(
                                    RichText::new(format!(
                                        "Darwin {}.{}.{} – {}.{}.{}",
                                        r.min.0, r.min.1, r.min.2, r.max.0, r.max.1, r.max.2
                                    ))
                                    .size(12.)
                                    .color(MUTED),
                                );
                            }
                        });
                        ui.label(
                            RichText::new(guide_ui::status_reason(d.status))
                                .size(13.)
                                .color(MUTED),
                        );
                        guide_ui::diagnostic(ui, std::iter::once(d.reason.as_str()));
                    });
            }
            ui.add_space(12.);
            egui::CollapsingHeader::new(tf(
                "hardware.notes",
                &[("0", format!("{}", a.warnings.len()))],
            ))
            .default_open(true)
            .show(ui, |ui| {
                for w in &a.warnings {
                    ui.label(RichText::new(w).color(WARNING));
                }
            });
        } else {
            empty(ui, tr("report.empty"), tr("report.empty_help"));
        }
    }
    fn efi(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.build_workflow(ui, ctx);
        ui.separator();
        ui.label(tr("efi.folder_hint"));
        let busy = self.worker.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new(tr("action.audit_efi")))
                .clicked()
            {
                self.start(ctx, || {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        Ok(Message::Audit(efi::audit(&path, None)?))
                    } else {
                        Ok(Message::Saved(tr("task.cancelled").into()))
                    }
                });
            }
            if ui
                .add_enabled(
                    !busy && self.audit.is_some(),
                    egui::Button::new(tr("action.repair_efi")),
                )
                .clicked()
                && let Some(a) = &self.audit
            {
                let root = a.root.clone();
                let output_root = self.root.clone();
                self.start(ctx, move || {
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_nanos();
                    let output = output_root.join(format!("efi-repair-{stamp}"));
                    let changes = efi::repair_copy(&root, &output)?;
                    workspace::save_new(&output_root, "policy-changes.json", &changes)?;
                    Ok(Message::Saved(tf(
                        "efi.copy_result",
                        &[
                            ("0", format!("{}", output.display())),
                            ("1", format!("{}", changes.len())),
                        ],
                    )))
                });
            }
        });
        ui.add_space(18.);
        if let Some(a) = &self.audit {
            ui.monospace(a.root.display().to_string());
            ui.label(tf("efi.files", &[("0", format!("{}", a.files))]));
            ui.monospace(format!("config SHA-256: {}", a.config_sha256));
            if !a.issues.is_empty() {
                ui.label(
                    RichText::new(tf("efi.issues", &[("0", a.issues.len().to_string())]))
                        .color(ERROR),
                );
            }
            guide_ui::diagnostic(ui, a.issues.iter().chain(&a.warnings).map(String::as_str));
            if a.issues.is_empty() {
                ui.label(RichText::new(tr("efi.structure_ok")).color(ACCENT));
            }
        } else {
            empty(ui, tr("efi.copy_title"), tr("efi.copy_help"));
        }
        ui.add_space(20.);
        ui.label(RichText::new(tr("efi.after_build")).color(MUTED));
    }
    fn usb(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.usb_workflow(ui, ctx);
        ui.separator();
        if ui
            .add_enabled(
                self.worker.is_none(),
                egui::Button::new(tr("action.refresh_drives")),
            )
            .clicked()
        {
            self.start(ctx, || Ok(Message::Disks(disks::list()?)));
        }
        ui.label(RichText::new(tr("usb.protected_hint")).color(WARNING));
        ui.add_space(12.);
        for disk in &self.disks {
            egui::Frame::new()
                .fill(PANEL)
                .corner_radius(3)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width() - 24.);
                    ui.horizontal(|ui| {
                        ui.strong(if disk.model.is_empty() {
                            &disk.path
                        } else {
                            &disk.model
                        });
                        ui.label(format!(
                            "{:.1} GiB · {}",
                            disk.size as f64 / 1073741824.,
                            disk.bus
                        ));
                    });
                    ui.monospace(&disk.path);
                    ui.label(
                        RichText::new(tf("usb.serial", &[("0", disk.serial.to_string())]))
                            .color(MUTED),
                    );
                    if disk.reasons.is_empty() {
                        ui.label(RichText::new(tr("usb.eligible")).color(ACCENT));
                    }
                    for reason in &disk.reasons {
                        ui.label(RichText::new(reason).color(WARNING));
                    }
                });
        }
        if self.disks.is_empty() {
            empty(ui, tr("usb.empty"), tr("usb.empty_help"));
        }
    }
    fn logs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui
            .add_enabled(
                self.worker.is_none(),
                egui::Button::new(tr("action.save_log")),
            )
            .clicked()
        {
            let log = self.log.clone();
            let root = self.root.clone();
            self.start(ctx, move || {
                Ok(Message::Saved(tf(
                    "log.saved",
                    &[(
                        "0",
                        format!(
                            "{}",
                            workspace::save_new(&root, "log.json", &log)?.display()
                        ),
                    )],
                )))
            });
        }
        for line in &self.log {
            ui.monospace(line);
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.strong(tr("settings.language"));
        self.language_picker(ui);
        ui.label(RichText::new(tr("settings.language_hint")).color(MUTED));
        ui.add_space(18.);
        ui.strong(tr("settings.workspace"));
        ui.monospace(self.root.display().to_string());
        if ui.button(tr("action.change_folder")).clicked()
            && let Some(p) = rfd::FileDialog::new().pick_folder()
        {
            self.root = p;
        }
        ui.add_space(18.);
        ui.strong(tr("settings.appearance"));
        ui.label(tr("settings.appearance_detail"));
        ui.add_space(18.);
        ui.strong(tr("settings.about"));
        ui.label(format!(
            "Ferrite {} · Rust · Windows + Linux + macOS",
            ferrite_core::VERSION
        ));
        ui.label(tr("settings.privacy"));
        ui.hyperlink_to(tr("settings.source"), "https://github.com/notnotshuts/1401");
        ui.hyperlink_to(
            tr("settings.licenses"),
            "https://github.com/notnotshuts/1401/blob/main/NOTICE.md",
        );
    }
}
fn stat(ui: &mut egui::Ui, label: &str, value: &str, detail: &str) {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(3)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width() - 24.);
            ui.label(RichText::new(label).size(11.).color(MUTED));
            ui.label(RichText::new(value).size(20.).strong());
            ui.label(RichText::new(detail).size(12.).color(MUTED));
        });
}
fn empty(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(16.);
    ui.label(RichText::new(title).size(20.).strong());
    ui.label(RichText::new(detail).color(MUTED));
}
fn status_color(status: Status) -> Color32 {
    match status {
        Status::Supported => ACCENT,
        Status::Driver | Status::Patched => WARNING,
        Status::Unsupported => ERROR,
        Status::Unknown => MUTED,
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if self.worker.is_none() {
            for file in files {
                if let Some(path) = file.path {
                    self.import(path);
                    self.page = Page::Hardware;
                }
            }
        }
        self.editor_shell(ctx);
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Ferrite",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1240., 820.])
                .with_min_inner_size([760., 580.])
                .with_title("Ferrite — OpenCore"),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

mod workflow_ui;

mod guide_ui;
mod preferences;

mod editor_ui;
