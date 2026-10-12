use crate::*;
const PAGES: [Page; 7] = [
    Page::Overview,
    Page::Hardware,
    Page::Guide,
    Page::Efi,
    Page::Usb,
    Page::Log,
    Page::Settings,
];
impl App {
    pub(crate) fn editor_shell(&mut self, ctx: &egui::Context) {
        let palette_shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::P,
        );
        if ctx.input_mut(|i| i.consume_shortcut(&palette_shortcut)) {
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::W,
            ))
        }) && self.tabs.len() > 1
        {
            self.tabs.retain(|p| *p != self.page);
            self.page = *self.tabs.last().unwrap_or(&Page::Overview);
        }
        if !self.tabs.contains(&self.page) {
            self.tabs.push(self.page);
        }
        let narrow = ctx.screen_rect().width() < 950.;
        egui::TopBottomPanel::top("workspace-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Ferrite").strong());
                    ui.label(RichText::new("/  OpenCore").color(MUTED).size(12.));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.language_picker(ui);
                        if ui
                            .button(tr("editor.commands"))
                            .on_hover_text(if cfg!(target_os = "macos") {
                                "⌘⇧P"
                            } else {
                                "Ctrl+Shift+P"
                            })
                            .clicked()
                        {
                            self.palette_open = true;
                            self.palette_query.clear();
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ctx, |ui| {
                if let Some(p) = self.progress {
                    ui.add(egui::ProgressBar::new(p.clamp(0., 1.)).desired_height(3.));
                }
                ui.horizontal(|ui| {
                    if self.worker.is_some() {
                        ui.spinner();
                    } else {
                        ui.label(RichText::new("•").color(ACCENT));
                    }
                    ui.label(
                        RichText::new(if self.notice.is_empty() {
                            tr("task.ready")
                        } else {
                            &self.notice
                        })
                        .size(11.)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}  ·  {}",
                                ferrite_core::platform::host_name(),
                                ferrite_core::VERSION
                            ))
                            .size(11.)
                            .color(MUTED),
                        );
                    });
                });
            });
        egui::SidePanel::left("navigation")
            .resizable(true)
            .default_width(if narrow { 174. } else { 210. })
            .width_range(160. ..=300.)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(10))
            .show(ctx, |ui| {
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("editor.project")).size(11.).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("+")
                            .on_hover_text(tr("action.import"))
                            .clicked()
                            && self.worker.is_none()
                        {
                            self.choose_report(ctx);
                        }
                    });
                });
                ui.add_space(10.);
                ui.label(RichText::new("⌄  ferrite").monospace().strong());
                ui.add_space(4.);
                ui.indent("project-tree", |ui| {
                    for (index, page) in PAGES.into_iter().enumerate() {
                        let selected = self.page == page;
                        let text = format!("{:02}  {}", index + 1, page.title());
                        let button = egui::Button::new(RichText::new(text).color(if selected {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .fill(if selected { CARD } else { PANEL })
                        .stroke(egui::Stroke::NONE)
                        .min_size(Vec2::new(ui.available_width(), 29.));
                        if ui.add(button).clicked() {
                            self.page = page;
                        }
                    }
                });
                ui.add_space(18.);
                ui.separator();
                ui.add_space(8.);
                ui.label(RichText::new(tr("editor.report")).size(11.).color(MUTED));
                if self.source.is_empty() {
                    ui.label(RichText::new(tr("editor.no_report")).size(12.).color(MUTED));
                } else {
                    let filename = std::path::Path::new(&self.source)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    ui.label(RichText::new(filename).monospace().size(12.));
                    if let Some(report) = &self.report {
                        ui.label(
                            RichText::new(format!(
                                "SHA  {}",
                                &report.sha256[..report.sha256.len().min(10)]
                            ))
                            .monospace()
                            .size(11.)
                            .color(MUTED),
                        );
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    if ui.button(tr("action.open_guide")).clicked() {
                        self.page = Page::Guide;
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(4, 0))
                    .show(ui, |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("editor-tabs")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for page in self.tabs.clone() {
                                        let active = self.page == page;
                                        let frame = egui::Frame::new()
                                            .fill(if active { BG } else { PANEL })
                                            .inner_margin(egui::Margin::symmetric(10, 7));
                                        let response = frame
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new(page.title())
                                                                    .size(12.)
                                                                    .color(if active {
                                                                        TEXT
                                                                    } else {
                                                                        MUTED
                                                                    }),
                                                            )
                                                            .frame(false),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.page = page;
                                                    }
                                                    if self.tabs.len() > 1
                                                        && ui.small_button("×").clicked()
                                                    {
                                                        self.tabs.retain(|p| *p != page);
                                                        if self.page == page {
                                                            self.page = *self
                                                                .tabs
                                                                .last()
                                                                .unwrap_or(&Page::Overview);
                                                        }
                                                    }
                                                });
                                            })
                                            .response;
                                        if active {
                                            ui.painter().line_segment(
                                                [
                                                    response.rect.left_bottom(),
                                                    response.rect.right_bottom(),
                                                ],
                                                egui::Stroke::new(2_f32, ACCENT),
                                            );
                                        }
                                    }
                                });
                            });
                    });
                ui.add_space(12.);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(if narrow { 16 } else { 24 }, 0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("ferrite  /  {}", self.page.title()))
                                    .size(11.)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(12.);
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width().min(1020.));
                                if self.page != Page::Overview {
                                    ui.label(RichText::new(self.page.title()).size(20.).strong());
                                    ui.add_space(12.);
                                }
                                match self.page {
                                    Page::Overview => self.overview(ui, ctx),
                                    Page::Hardware => self.hardware(ui, ctx),
                                    Page::Guide => self.guide(ui),
                                    Page::Efi => self.efi(ui, ctx),
                                    Page::Usb => self.usb(ui, ctx),
                                    Page::Log => self.logs(ui, ctx),
                                    Page::Settings => self.settings(ui, ctx),
                                }
                                ui.add_space(20.);
                            });
                    });
            });
        self.command_palette(ctx);
    }
    fn command_palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.palette_open = false;
            return;
        }
        #[derive(Clone, Copy)]
        enum Action {
            Page(Page),
            Scan,
            Import,
        }
        let mut commands: Vec<(String, Action)> = PAGES
            .into_iter()
            .map(|p| (p.title().into(), Action::Page(p)))
            .collect();
        if self.worker.is_none() {
            commands.push((tr("action.scan_pc").into(), Action::Scan));
            commands.push((tr("action.import").into(), Action::Import));
        }
        let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut chosen = None;
        let mut open = self.palette_open;
        egui::Window::new(tr("editor.commands"))
            .id(egui::Id::new("command-palette"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fade_in(false)
            .fade_out(false)
            .default_width(500.)
            .anchor(egui::Align2::CENTER_TOP, [0., 80.])
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text(tr("editor.search_commands"))
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    self.palette_index = 0;
                }
                response.request_focus();
                ui.separator();
                let q = self.palette_query.to_lowercase();
                let filtered: Vec<_> = commands
                    .iter()
                    .filter(|(title, _)| title.to_lowercase().contains(&q))
                    .collect();
                if !filtered.is_empty() {
                    self.palette_index = self.palette_index.min(filtered.len() - 1);
                    if down {
                        self.palette_index = (self.palette_index + 1) % filtered.len();
                    }
                    if up {
                        self.palette_index =
                            (self.palette_index + filtered.len() - 1) % filtered.len();
                    }
                    if enter {
                        chosen = Some(filtered[self.palette_index].1);
                    }
                }
                for (i, (title, action)) in filtered.into_iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 28.),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title)
                    });
                    if self.palette_index == i || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            3,
                            if self.palette_index == i {
                                ACCENT.gamma_multiply(0.18)
                            } else {
                                CARD
                            },
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + Vec2::new(10., 0.),
                        egui::Align2::LEFT_CENTER,
                        title,
                        egui::FontId::proportional(13.),
                        TEXT,
                    );
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        chosen = Some(*action);
                    }
                }
            });
        self.palette_open = open;
        if let Some(action) = chosen {
            self.palette_open = false;
            self.palette_index = 0;
            match action {
                Action::Page(page) => self.page = page,
                Action::Scan => self.scan(ctx),
                Action::Import => self.choose_report(ctx),
            }
        }
    }
}
