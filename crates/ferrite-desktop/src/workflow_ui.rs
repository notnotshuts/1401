use crate::*;
use anyhow::Context;
use std::fs;
impl App {
    fn preview_plan(&self) -> anyhow::Result<Plan> {
        let report = self.report.as_ref().context(tr("build.report_required"))?;
        let mut options = self.options.clone();
        options.target = self.target;
        planner::plan(report, &options)
    }
    pub(crate) fn build_workflow(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new(tr("build.title")).size(19.).strong());
        ui.label(RichText::new(tr("build.subtitle")).color(MUTED));
        let busy = self.worker.is_some();
        let previous = serde_json::to_string(&self.options).unwrap_or_default();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                self.version(ui);
                if ui.button(tr("action.choose_acpi")).clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                {
                    self.acpi_folder = Some(path);
                    self.build_result = None;
                }
            });
            if let Some(path) = &self.acpi_folder {
                ui.label(
                    RichText::new(format!("ACPI: {}", path.display()))
                        .size(12.)
                        .color(MUTED),
                );
            } else {
                ui.label(RichText::new(tr("build.acpi_required")).color(WARNING));
            }
            egui::CollapsingHeader::new(tr("build.advanced")).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("iasl");
                    ui.text_edit_singleline(&mut self.iasl);
                });
                ui.label(RichText::new(tr("build.iasl_help")).size(12.).color(MUTED));
                ui.checkbox(&mut self.options.verbose, tr("build.verbose"));
                if let Some(report) = &self.report {
                    let gpus = report.rows("GPU");
                    if gpus.len() > 1 {
                        ui.label(tr("build.gpu_choice"));
                        for (name, _) in gpus {
                            let mut enabled = !self.options.disabled.iter().any(|n| n == name);
                            if ui.checkbox(&mut enabled, name).changed() {
                                self.options.disabled.retain(|n| n != name);
                                if !enabled {
                                    self.options.disabled.push(name.into());
                                }
                            }
                        }
                    }
                }

                ui.horizontal(|ui| {
                    ui.label("Intel Wi-Fi");
                    egui::ComboBox::from_id_salt("wifi-choice")
                        .selected_text(if self.options.intel_wifi == "auto" {
                            tr("choice.auto")
                        } else {
                            &self.options.intel_wifi
                        })
                        .show_ui(ui, |ui| {
                            for value in ["auto", "itlwm", "AirportItlwm"] {
                                ui.selectable_value(
                                    &mut self.options.intel_wifi,
                                    value.into(),
                                    if value == "auto" {
                                        tr("choice.auto")
                                    } else {
                                        value
                                    },
                                );
                            }
                        });
                });
                ui.label(RichText::new(tr("build.wifi_help")).size(12.).color(MUTED));
            });
        });
        if previous != serde_json::to_string(&self.options).unwrap_or_default() {
            self.build_result = None;
            self.recovery_result = None;
        }
        match self.preview_plan() {
            Ok(plan) => {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1_f32, BORDER))
                    .corner_radius(3)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!("SMBIOS {}", plan.smbios));
                            ui.label(tf(
                                "build.summary",
                                &[
                                    ("0", format!("{}", plan.kexts.len())),
                                    (
                                        "1",
                                        format!(
                                            "{:.1}",
                                            plan.artifacts.iter().map(|a| a.size).sum::<u64>()
                                                as f64
                                                / 1048576.
                                        ),
                                    ),
                                ],
                            ));
                        });
                        egui::CollapsingHeader::new(tr("build.details")).show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    plan.kexts
                                        .iter()
                                        .map(|k| k.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                )
                                .size(12.)
                                .color(MUTED),
                            );
                        });
                        if !plan.disabled.is_empty() {
                            ui.label(
                                RichText::new(tf(
                                    "build.disabled",
                                    &[("0", plan.disabled.join(", ").to_string())],
                                ))
                                .color(WARNING),
                            );
                        }
                    });
                if ui
                    .add_enabled(
                        !busy && self.acpi_folder.is_some(),
                        egui::Button::new(RichText::new(tr("action.build_efi")).color(BG))
                            .fill(ACCENT),
                    )
                    .clicked()
                {
                    let report = PathBuf::from(&self.source);
                    let acpi = self.acpi_folder.clone().unwrap_or_default();
                    let root = self.root.clone();
                    let iasl = self.iasl.clone();
                    let mut options = self.options.clone();
                    options.target = self.target;
                    self.start_stream(ctx, move |sender| {
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_nanos();
                        let output = root.join(format!("build-{stamp}"));
                        let cache = root.join("cache");
                        let build = builder::build(
                            &report,
                            &acpi,
                            &output,
                            &cache,
                            &iasl,
                            &options,
                            &mut |n, d, t| {
                                let _ = sender.send(Message::Progress(n.into(), d, t));
                            },
                        )?;
                        Ok(Message::Build(Box::new(build)))
                    });
                }
            }
            Err(error) => {
                ui.label(
                    RichText::new(tr(if self.report.is_some() {
                        "build.plan_unavailable"
                    } else {
                        "build.report_required"
                    }))
                    .color(WARNING),
                );
                guide_ui::diagnostic(ui, std::iter::once(error.to_string().as_str()));
            }
        }
        if let Some(build) = &self.build_result {
            ui.add_space(14.);
            ui.label(RichText::new(tr("build.ready")).color(ACCENT));
            ui.monospace(build.output.display().to_string());
            if ui.button(tr("action.open_guide")).clicked() {
                self.page = Page::Guide;
            }
            ui.label(
                RichText::new(tf(
                    "build.generated",
                    &[("0", build.acpi.generated.join(", ").to_string())],
                ))
                .size(12.)
                .color(MUTED),
            );
            egui::CollapsingHeader::new(tr("build.check_after")).show(ui, |ui| {
                ui.label(tr("build.review_required"));
                guide_ui::diagnostic(
                    ui,
                    build
                        .plan
                        .notices
                        .iter()
                        .chain(&build.acpi.notices)
                        .map(String::as_str),
                );
            });
            if ui
                .add_enabled(!busy, egui::Button::new(tr("action.next_recovery")))
                .clicked()
            {
                let target = build.plan.target;
                let output = build.output.join("Recovery");
                self.start_stream(ctx, move |sender| {
                    let info = recovery::info(target)?;
                    let result = recovery::download(&info, &output, &mut |n, d, t| {
                        let _ = sender.send(Message::Progress(n.into(), d, t));
                    })?;
                    Ok(Message::Recovery(result))
                });
            }
        }
        ui.add_space(18.);
    }
    pub(crate) fn usb_workflow(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new(tr("usb.title")).size(19.).strong());
        let busy = self.worker.is_some();
        if !ferrite_core::platform::physical_usb_supported() {
            ui.label(RichText::new(tr("mac.usb_image_only")).color(MUTED));
        }
        if self.build_result.is_none() {
            ui.label(RichText::new(tr("usb.build_required")).color(MUTED));
            return;
        }
        if self.recovery_result.is_none() {
            ui.label(RichText::new(tr("usb.recovery_required")).color(MUTED));
            if ui
                .add_enabled(!busy, egui::Button::new(tr("action.download_recovery")))
                .clicked()
            {
                let build = self.build_result.as_ref().unwrap();
                let target = build.plan.target;
                let output = build.output.join("Recovery");
                self.start_stream(ctx, move |sender| {
                    let info = recovery::info(target)?;
                    Ok(Message::Recovery(recovery::download(
                        &info,
                        &output,
                        &mut |n, d, t| {
                            let _ = sender.send(Message::Progress(n.into(), d, t));
                        },
                    )?))
                });
            }
            return;
        }
        let recovery = self.recovery_result.as_ref().unwrap();
        ui.label(
            RichText::new(tf(
                "recovery.verified",
                &[
                    ("0", format!("{:.1}", recovery.bytes as f64 / 1048576.)),
                    ("1", format!("{}", recovery.chunks)),
                ],
            ))
            .color(ACCENT),
        );
        if ui
            .add_enabled(!busy, egui::Button::new(tr("action.refresh_drives")))
            .clicked()
        {
            self.start(ctx, || Ok(Message::Disks(disks::list()?)));
        }
        for disk in &self.disks {
            ui.add_enabled_ui(disk.eligible() && !busy, |ui| {
                ui.radio_value(
                    &mut self.selected_disk,
                    Some(disk.path.clone()),
                    format!(
                        "{} · {:.1} GiB · {}",
                        disk.path,
                        disk.size as f64 / 1073741824.,
                        disk.model
                    ),
                );
            });
        }
        let selected = self
            .disks
            .iter()
            .find(|d| self.selected_disk.as_deref() == Some(d.path.as_str()))
            .cloned();
        if let Some(disk) = selected {
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(3)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("usb.erase_warning")).strong().color(ERROR));
                    ui.label(tf(
                        "usb.identity",
                        &[("0", disk.path.to_string()), ("1", disk.serial.to_string())],
                    ));
                    ui.label(tr("usb.confirm_hint"));
                    ui.text_edit_singleline(&mut self.confirmation);
                });
            let ready = !busy && disk.eligible() && self.confirmation == disk.path;
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(tr("action.write_usb")).fill(Color32::from_rgb(108, 46, 53)),
                )
                .clicked()
            {
                let build = self.build_result.as_ref().unwrap().output.clone();
                let recovery = self
                    .recovery_result
                    .as_ref()
                    .unwrap()
                    .image
                    .parent()
                    .unwrap()
                    .to_path_buf();
                let root = self.root.clone();
                let confirm = self.confirmation.clone();
                self.start(ctx, move || {
                    let request = workspace::save_new(&root, "selected-disk.json", &disk)?;
                    let result = request
                        .parent()
                        .context("USB request")?
                        .join("write-result.json");
                    let cli = std::env::current_exe()?
                        .parent()
                        .context("App directory")?
                        .join(if cfg!(target_os = "windows") {
                            "ferrite-cli.exe"
                        } else {
                            "ferrite-cli"
                        });
                    if !cli.is_file() {
                        anyhow::bail!(tr("usb.cli_missing"));
                    }
                    let args = vec![
                        "write".into(),
                        build.display().to_string(),
                        recovery.display().to_string(),
                        "--selection".into(),
                        request.display().to_string(),
                        "--confirm".into(),
                        confirm,
                        "--result".into(),
                        result.display().to_string(),
                    ];
                    elevation::write_helper(&cli, &args)?;
                    let written: ferrite_core::usb::Written =
                        serde_json::from_slice(&fs::read(result)?)?;
                    if !written.verified {
                        anyhow::bail!(tr("usb.verify_failed"));
                    }
                    Ok(Message::Written(tf(
                        "usb.written",
                        &[
                            ("0", written.device.to_string()),
                            ("1", format!("{}", written.files)),
                        ],
                    )))
                });
            }
        }
        if ui
            .add_enabled(!busy, egui::Button::new(tr("action.create_image")))
            .clicked()
        {
            let build = self.build_result.as_ref().unwrap().output.clone();
            let recovery = self
                .recovery_result
                .as_ref()
                .unwrap()
                .image
                .parent()
                .unwrap()
                .to_path_buf();
            if let Some(output) = rfd::FileDialog::new()
                .add_filter(tr("dialog.image"), &["img"])
                .set_file_name("Ferrite-Installer.img")
                .save_file()
            {
                self.start_stream(ctx, move |sender| {
                    let result =
                        ferrite_core::usb::image(&build, &recovery, &output, 8, &mut |n, d, t| {
                            let _ = sender.send(Message::Progress(n.into(), d, t));
                        })?;
                    Ok(Message::Written(tf(
                        "usb.image_ready",
                        &[("0", result.device.to_string())],
                    )))
                });
            }
        }
        ui.add_space(18.);
    }
}
