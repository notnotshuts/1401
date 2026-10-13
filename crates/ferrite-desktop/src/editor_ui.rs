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
        let colors = theme::colors(ctx.style().visuals.dark_mode);
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::P,
            ))
        }) {
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
        }
        let compact = ctx.screen_rect().width() < 900.;
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(colors.bg)
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Ferrite").size(22.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("···")
                            .on_hover_text(tr("editor.commands"))
                            .clicked()
                        {
                            self.palette_open = true;
                        }
                        if compact {
                            if ui.button(tr("nav.settings")).clicked() {
                                self.page = Page::Settings;
                            }
                            if ui.button(tr("nav.log")).clicked() {
                                self.page = Page::Log;
                            }
                        }
                    });
                });
                if compact {
                    ui.add_space(8.);
                    ui.horizontal_wrapped(|ui| {
                        for page in [
                            Page::Overview,
                            Page::Hardware,
                            Page::Efi,
                            Page::Usb,
                            Page::Guide,
                        ] {
                            ui.selectable_value(&mut self.page, page, page.title());
                        }
                    });
                }
            });
        if !compact {
            egui::SidePanel::left("navigation")
                .resizable(false)
                .exact_width(226.)
                .frame(
                    egui::Frame::new()
                        .fill(colors.bg)
                        .inner_margin(egui::Margin::symmetric(16, 12)),
                )
                .show(ctx, |ui| {
                    theme::glass(ui).inner_margin(12).show(ui, |ui| {
                        for page in [Page::Overview, Page::Hardware, Page::Efi, Page::Usb] {
                            if nav(ui, page, self.page == page, true).clicked() {
                                self.page = page;
                            }
                        }
                        ui.add_space(20.);
                        for page in [Page::Guide, Page::Log, Page::Settings] {
                            if nav(ui, page, self.page == page, true).clicked() {
                                self.page = page;
                            }
                        }
                    });
                    ui.add_space(16.);
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  {}",
                            ferrite_core::platform::host_name(),
                            ferrite_core::VERSION
                        ))
                        .size(12.)
                        .color(colors.muted),
                    );
                });
        }
        if self.worker.is_some() || !self.notice.is_empty() {
            egui::TopBottomPanel::bottom("activity")
                .frame(
                    egui::Frame::new()
                        .fill(colors.bg)
                        .inner_margin(egui::Margin::symmetric(24, 12)),
                )
                .show(ctx, |ui| {
                    theme::glass(ui).inner_margin(14).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if self.worker.is_some() {
                                ui.spinner();
                            }
                            ui.add(
                                egui::Label::new(RichText::new(&self.notice).size(14.)).truncate(),
                            )
                            .on_hover_text(&self.notice);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button(tr("nav.log")).clicked() {
                                        self.page = Page::Log;
                                    }
                                },
                            );
                        });
                        if let Some(progress) = self.progress {
                            ui.add(
                                egui::ProgressBar::new(progress.clamp(0., 1.)).desired_height(5.),
                            );
                        }
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.bg)
                    .inner_margin(egui::Margin::symmetric(28, 20)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(940.);
                        if self.page != Page::Overview {
                            ui.label(RichText::new(self.page.title()).size(34.).strong());
                            ui.add_space(20.);
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
                        ui.add_space(24.);
                    });
            });
        self.command_palette(ctx);
    }
    fn command_palette(&mut self, ctx: &egui::Context) {
        let colors = theme::colors(ctx.style().visuals.dark_mode);
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
                        Vec2::new(ui.available_width(), 44.),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title)
                    });
                    if self.palette_index == i || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            12,
                            if self.palette_index == i {
                                colors.accent.gamma_multiply(0.18)
                            } else {
                                colors.card
                            },
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + Vec2::new(10., 0.),
                        egui::Align2::LEFT_CENTER,
                        title,
                        egui::FontId::proportional(16.),
                        colors.text,
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

fn nav(ui: &mut egui::Ui, page: Page, selected: bool, full: bool) -> egui::Response {
    let c = theme::palette(ui);
    let width = if full {
        ui.available_width()
    } else {
        ui.fonts(|f| {
            f.layout_no_wrap(page.title().into(), egui::FontId::proportional(15.), c.text)
                .size()
                .x
        }) + 28.
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 46.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, page.title())
    });
    let hover = ui
        .ctx()
        .animate_bool(response.id.with("hover"), response.hovered());
    if selected || hover > 0.01 {
        ui.painter().rect_filled(
            rect,
            14,
            if selected {
                c.accent.gamma_multiply(0.14)
            } else {
                c.card.gamma_multiply(hover)
            },
        );
    }
    let color = if selected { c.accent } else { c.text };
    let x = if full {
        icon(
            ui.painter(),
            egui::Rect::from_min_size(rect.left_center() + Vec2::new(12., -10.), Vec2::splat(20.)),
            page,
            color,
        );
        44.
    } else {
        14.
    };
    ui.painter().text(
        rect.left_center() + Vec2::new(x, 0.),
        egui::Align2::LEFT_CENTER,
        page.title(),
        egui::FontId::proportional(15.),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn icon(painter: &egui::Painter, rect: egui::Rect, page: Page, color: Color32) {
    let stroke = egui::Stroke::new(1.7_f32, color);
    let point = |x: f32, y: f32| rect.min + Vec2::new(x, y);
    match page {
        Page::Overview => {
            for (x, y) in [(1., 1.), (12., 1.), (1., 12.), (12., 12.)] {
                painter.rect_stroke(
                    egui::Rect::from_min_size(point(x, y), Vec2::splat(7.)),
                    2,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Page::Hardware => {
            painter.rect_stroke(rect.shrink(4.), 3, stroke, egui::StrokeKind::Inside);
            for v in [6., 10., 14.] {
                for (a, b) in [
                    (point(v, 0.), point(v, 4.)),
                    (point(v, 16.), point(v, 20.)),
                    (point(0., v), point(4., v)),
                    (point(16., v), point(20., v)),
                ] {
                    painter.line_segment([a, b], stroke);
                }
            }
        }
        Page::Usb => {
            painter.rect_stroke(
                rect.shrink2(Vec2::new(5., 1.)),
                3,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([point(5., 8.), point(15., 8.)], stroke);
            painter.circle_filled(point(10., 15.), 1.3, color);
        }
        Page::Efi => {
            painter.circle_stroke(rect.center(), 8., stroke);
            painter.line_segment([point(10., 4.), point(10., 10.)], stroke);
            painter.line_segment([point(10., 10.), point(14., 13.)], stroke);
        }
        Page::Guide => {
            painter.rect_stroke(rect.shrink(1.), 3, stroke, egui::StrokeKind::Inside);
            painter.line_segment([point(10., 2.), point(10., 18.)], stroke);
        }
        Page::Log => {
            for y in [4., 10., 16.] {
                painter.line_segment([point(2., y), point(18., y)], stroke);
            }
        }
        Page::Settings => {
            painter.circle_stroke(rect.center(), 8., stroke);
            painter.circle_stroke(rect.center(), 3., stroke);
        }
    }
}
