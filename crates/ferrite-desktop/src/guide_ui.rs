use crate::*;
use ferrite_core::guide;
impl App {
    pub(crate) fn language_picker(&mut self, ui: &mut egui::Ui) {
        let previous = self.language;
        egui::ComboBox::from_id_salt("language")
            .selected_text(self.language.name())
            .show_ui(ui, |ui| {
                for language in Language::ALL {
                    ui.selectable_value(&mut self.language, language, language.name());
                }
            });
        if self.language != previous {
            i18n::set_language(self.language);
            self.notice = tr("task.ready").into();
            if let Err(error) = preferences::save(self.language) {
                self.notice = tr("settings.save_failed").into();
                self.log.push(error.to_string());
            }
        }
    }
    pub(crate) fn guide(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("guide.subtitle")).color(MUTED));
        if let Some(build) = &self.build_result {
            ui.label(tf(
                "guide.context",
                &[
                    ("0", compatibility::version_name(build.plan.target).into()),
                    ("1", build.plan.smbios.clone()),
                ],
            ));
        }
        if ui.button(tr("guide.export")).clicked() {
            let plan = self.build_result.as_ref().map(|b| &b.plan);
            let content = guide::render(plan, self.language);
            match workspace::new_session(&self.root).and_then(|path| {
                let output = path.join(format!("Guide-{}.md", self.language.code()));
                std::fs::write(&output, content)?;
                Ok(output)
            }) {
                Ok(path) => self.notice = tf("guide.saved", &[("0", path.display().to_string())]),
                Err(e) => {
                    self.notice = tr("task.failed").into();
                    self.log.push(e.to_string());
                }
            }
        }
        for (step, (title, body)) in guide::STEPS.iter().enumerate() {
            ui.add_space(12.);
            egui::Frame::new()
                .fill(PANEL)
                .corner_radius(3)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width() - 24.);
                    ui.label(RichText::new(tr(title)).size(18.).strong());
                    ui.label(tr(body));
                    if step == 1 {
                        ui.label(
                            RichText::new(tr("guide.bios.check_hint"))
                                .size(12.)
                                .color(MUTED),
                        );
                        let amd = self.report.as_ref().is_some_and(|r| {
                            r.hardware["CPU"]["Manufacturer"]
                                .as_str()
                                .is_some_and(|s| s.contains("AMD"))
                        });
                        let count = if amd { 7 } else { 8 };
                        for (i, key) in guide::BIOS.iter().take(count).enumerate() {
                            ui.checkbox(&mut self.bios_checks[i], tr(key));
                        }
                        ui.label(
                            RichText::new(tf(
                                "guide.bios.progress",
                                &[
                                    (
                                        "0",
                                        self.bios_checks[..count]
                                            .iter()
                                            .filter(|v| **v)
                                            .count()
                                            .to_string(),
                                    ),
                                    ("1", count.to_string()),
                                ],
                            ))
                            .color(MUTED),
                        );
                    }
                    let action = match step {
                        0 => Some(("guide.step1.action", Page::Hardware)),
                        2 => Some(("guide.step3.action", Page::Efi)),
                        3 | 4 => Some(("guide.step4.action", Page::Usb)),
                        _ => None,
                    };
                    if let Some((key, page)) = action
                        && ui.button(tr(key)).clicked()
                    {
                        self.page = page;
                    }
                    if step == 4 {
                        ui.label(RichText::new(tr("guide.step5.warning")).color(WARNING));
                    }
                    if step == 5 {
                        if self.build_result.as_ref().is_some_and(|b| b.plan.nvidia) {
                            ui.label(RichText::new(tr("guide.nvidia")).color(WARNING));
                        }
                        for (i, key) in guide::CHECKS.iter().enumerate() {
                            ui.checkbox(&mut self.install_checks[i], tr(key));
                        }
                    }
                });
        }
        ui.add_space(16.);
        egui::CollapsingHeader::new(tr("guide.troubleshoot.title")).show(ui, |ui| {
            ui.label(tr("guide.troubleshoot.body"));
        });
    }
}
pub(crate) fn diagnostic<'a>(ui: &mut egui::Ui, messages: impl Iterator<Item = &'a str>) {
    egui::CollapsingHeader::new(tr("details.title")).show(ui, |ui| {
        ui.label(RichText::new(tr("details.hint")).size(12.).color(MUTED));
        for message in messages {
            ui.label(RichText::new(message).size(12.).color(MUTED));
        }
    });
}
pub(crate) fn status_label(status: Status) -> &'static str {
    tr(match status {
        Status::Supported => "status.supported",
        Status::Driver => "status.driver",
        Status::Patched => "status.patched",
        Status::Unsupported => "status.unsupported",
        Status::Unknown => "status.unknown",
    })
}
pub(crate) fn status_reason(status: Status) -> &'static str {
    tr(match status {
        Status::Supported => "reason.supported",
        Status::Driver => "reason.driver",
        Status::Patched => "reason.patched",
        Status::Unsupported => "reason.unsupported",
        Status::Unknown => "reason.unknown",
    })
}
pub(crate) fn section_label(section: &str) -> &str {
    match section {
        "CPU" => tr("section.CPU"),
        "GPU" => tr("section.GPU"),
        "Network" => tr("section.Network"),
        "Storage Controllers" => tr("section.Storage Controllers"),
        "Sound" => tr("section.Sound"),
        "Bluetooth" => tr("section.Bluetooth"),
        "SD Controller" => tr("section.SD Controller"),
        "Input" => tr("section.Input"),
        "USB Controllers" => tr("section.USB Controllers"),
        _ => section,
    }
}
