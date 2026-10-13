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
    appearance: theme::Appearance,
    applied_dark: Option<bool>,
    palette_open: bool,
    palette_query: String,
    palette_index: usize,
    #[cfg(feature = "screenshot")]
    capture_frames: u8,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::fonts(&cc.egui_ctx);
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
            appearance: preferences::load_appearance(),
            applied_dark: None,
            palette_open: false,
            palette_query: String::new(),
            palette_index: 0,
            #[cfg(feature = "screenshot")]
            capture_frames: 0,
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
        #[cfg(feature = "screenshot")]
        {
            app.appearance = match std::env::var("FERRITE_PREVIEW_THEME").as_deref() {
                Ok("dark") => theme::Appearance::Dark,
                _ => theme::Appearance::Light,
            };
            match std::env::var("FERRITE_PREVIEW_PAGE").as_deref() {
                Ok("usb") => app.page = Page::Usb,
                Ok("settings") => app.page = Page::Settings,
                _ => {}
            }
        }
        #[cfg(feature = "screenshot")]
        if std::env::var("FERRITE_PREVIEW_DEMO").as_deref() == Ok("1") {
            preview::populate(&mut app);
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
        #[cfg(feature = "screenshot")]
        if std::env::var_os("FERRITE_SCREENSHOT_TO").is_some()
            || std::env::var("FERRITE_PREVIEW_DEMO").as_deref() == Ok("1")
        {
            return;
        }
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
                    self.notice = if name.is_empty() {
                        tr("task.running").into()
                    } else {
                        name
                    };
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
        let colors = theme::palette(ui);
        ui.label(RichText::new(tr("ui.welcome")).size(34.).strong());
        ui.label(RichText::new(tr("ui.welcome_hint")).color(colors.muted));
        ui.add_space(20.);
        theme::card(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(if self.report.is_some() {
                    tr("ui.report_ready")
                } else {
                    tr("ui.start")
                })
                .size(23.)
                .strong(),
            );
            ui.add_space(6.);
            if self.report.is_none() {
                ui.label(RichText::new(tr("ui.report_hint")).color(colors.muted));
                ui.add_space(12.);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.worker.is_none(),
                            theme::primary(tr("action.scan_pc"), ui),
                        )
                        .clicked()
                    {
                        self.scan(ctx);
                    }
                    if ui
                        .add_enabled(
                            self.worker.is_none(),
                            egui::Button::new(tr("action.import")),
                        )
                        .clicked()
                    {
                        self.choose_report(ctx);
                    }
                });
                if cfg!(target_os = "macos") {
                    ui.add_space(8.);
                    ui.label(
                        RichText::new(tr("ui.mac_target"))
                            .size(13.)
                            .color(colors.muted),
                    );
                }
            } else {
                if let Some(a) = &self.analysis {
                    ui.label(
                        RichText::new(if a.ok {
                            tr("hardware.compatible")
                        } else {
                            tr("summary.limits")
                        })
                        .color(if a.ok {
                            colors.accent
                        } else {
                            colors.warning
                        }),
                    );
                }
                ui.add_space(12.);
                if ui.add(theme::primary(tr("ui.continue"), ui)).clicked() {
                    self.page = if self.build_result.is_some() {
                        Page::Usb
                    } else if self.analysis.as_ref().is_some_and(|a| a.ok) {
                        Page::Efi
                    } else {
                        Page::Hardware
                    };
                }
            }
        });
        ui.add_space(20.);
        let report_ready = self.report.is_some();
        let build_ready = self.build_result.is_some();
        let recovery_ready = self.recovery_result.is_some();
        for (number, title, detail, page, done) in [
            (
                "1",
                tr("nav.hardware"),
                tr("ui.step_hardware"),
                Page::Hardware,
                report_ready,
            ),
            (
                "2",
                "OpenCore EFI",
                tr("ui.step_efi"),
                Page::Efi,
                build_ready,
            ),
            (
                "3",
                tr("nav.usb"),
                tr("ui.step_usb"),
                Page::Usb,
                recovery_ready,
            ),
        ] {
            theme::card(ui).inner_margin(16).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(number).size(25.).color(colors.accent));
                    ui.vertical(|ui| {
                        ui.strong(title);
                        ui.label(RichText::new(detail).size(14.).color(colors.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(if done {
                                tr("ui.review")
                            } else {
                                tr("action.open")
                            })
                            .clicked()
                        {
                            self.page = page;
                        }
                    });
                });
            });
        }
    }
    fn hardware(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let colors = theme::palette(ui);
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
                    .margin(Vec2::new(12., 12.))
                    .desired_width(360.),
            );
            ui.checkbox(&mut self.only_problems, tr("hardware.only_limits"));
        });
        ui.add_space(8.);
        if let Some(a) = &self.analysis {
            if a.ok {
                ui.label(RichText::new(tr("hardware.compatible")).color(colors.accent));
            }
            if !a.blockers.is_empty() {
                ui.label(RichText::new(tr("hardware.blocked")).color(colors.error));
                guide_ui::diagnostic(ui, a.blockers.iter().map(String::as_str));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("hardware.native_versions"));
                for v in &a.native_versions {
                    ui.label(RichText::new(compatibility::version_name(*v)).color(colors.accent));
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
                    .fill(colors.panel)
                    .corner_radius(18)
                    .inner_margin(20)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width() - 24.);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(guide_ui::section_label(&d.section))
                                    .size(12.)
                                    .color(colors.muted),
                            );
                            ui.label(
                                RichText::new(guide_ui::status_label(d.status))
                                    .color(status_color(ui, d.status)),
                            );
                        });
                        ui.strong(&d.name);
                        egui::CollapsingHeader::new(tr("details.title"))
                            .id_salt((&d.section, &d.name, &d.id))
                            .show(ui, |ui| {
                                ui.monospace(&d.id);
                                ui.label(&d.reason);
                            });
                    });
            }
            ui.add_space(12.);
            egui::CollapsingHeader::new(tf(
                "hardware.notes",
                &[("0", format!("{}", a.warnings.len()))],
            ))
            .default_open(false)
            .show(ui, |ui| {
                for w in &a.warnings {
                    ui.label(RichText::new(w).color(colors.warning));
                }
            });
        } else {
            empty(ui, tr("report.empty"), tr("report.empty_help"));
        }
    }
    fn efi(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let colors = theme::palette(ui);
        self.build_workflow(ui, ctx);
        ui.add_space(16.);
        egui::CollapsingHeader::new(tr("ui.existing_efi")).show(ui, |ui| {
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
                            .color(colors.error),
                    );
                }
                guide_ui::diagnostic(ui, a.issues.iter().chain(&a.warnings).map(String::as_str));
                if a.issues.is_empty() {
                    ui.label(RichText::new(tr("efi.structure_ok")).color(colors.accent));
                }
            } else {
                empty(ui, tr("efi.copy_title"), tr("efi.copy_help"));
            }
            ui.add_space(20.);
        });
    }
    fn usb(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.usb_workflow(ui, ctx);
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
        let previous = self.appearance;
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (theme::Appearance::System, "ui.theme_system"),
                (theme::Appearance::Light, "ui.theme_light"),
                (theme::Appearance::Dark, "ui.theme_dark"),
            ] {
                ui.selectable_value(&mut self.appearance, mode, tr(key));
            }
        });
        if self.appearance != previous
            && let Err(error) = preferences::save(self.language, self.appearance)
        {
            self.notice = tr("settings.save_failed").into();
            self.log.push(error.to_string());
        }
        ui.add_space(18.);
        ui.strong(tr("settings.about"));
        ui.label(format!(
            "Ferrite {} · Rust · Windows + Linux + macOS",
            ferrite_core::VERSION
        ));

        ui.hyperlink_to(tr("settings.source"), "https://github.com/notnotshuts/1401");
        ui.hyperlink_to(
            tr("settings.licenses"),
            "https://github.com/notnotshuts/1401/blob/main/NOTICE.md",
        );
    }
}
fn empty(ui: &mut egui::Ui, title: &str, detail: &str) {
    let colors = theme::palette(ui);
    ui.add_space(16.);
    ui.label(RichText::new(title).size(20.).strong());
    ui.label(RichText::new(detail).color(colors.muted));
}
fn status_color(ui: &egui::Ui, status: Status) -> Color32 {
    let colors = theme::palette(ui);
    match status {
        Status::Supported => colors.accent,
        Status::Driver | Status::Patched => colors.warning,
        Status::Unsupported => colors.error,
        Status::Unknown => colors.muted,
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dark = match self.appearance {
            theme::Appearance::Light => false,
            theme::Appearance::Dark => true,
            theme::Appearance::System => ctx.system_theme() == Some(egui::Theme::Dark),
        };
        if self.applied_dark != Some(dark) {
            theme::apply(ctx, dark);
            self.applied_dark = Some(dark);
        }
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
        #[cfg(feature = "screenshot")]
        preview::capture(self, ctx);
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Ferrite",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(window_size())
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
mod theme;

fn window_size() -> [f32; 2] {
    #[cfg(feature = "screenshot")]
    if std::env::var("FERRITE_PREVIEW_COMPACT").as_deref() == Ok("1") {
        return [780., 720.];
    }
    [1240., 820.]
}
#[cfg(feature = "screenshot")]
mod preview;
