//! Screenshot-only synthetic state. This module is absent from release builds.
use crate::*;
pub fn populate(app: &mut App) {
    let report =
        Report::from_bytes(include_bytes!("../../../tools/ui/Report.json")).expect("UI fixture");
    app.root = "Demo/Workspace".into();
    app.analysis = Some(compatibility::analyze(&report, app.target));
    app.report = Some(report);
    app.source = "Demo/Report.json".into();
    app.notice = tr("ui.preview").into();
    if app.page == Page::Usb {
        app.disks = vec![Disk {
            path: "/dev/disk99".into(),
            model: "USB Drive".into(),
            serial: "DEMO-USB".into(),
            size: 32 * 1073741824,
            bus: "USB".into(),
            reasons: vec![],
            device_id: "preview-only".into(),
            sector_size: 512,
        }];
        app.selected_disk = Some("/dev/disk99".into());
        let plan = Plan {
            report_sha256: String::new(),
            target: 24,
            smbios: "iMac20,1".into(),
            kexts: vec![],
            artifacts: vec![],
            disabled: vec![],
            needs_oclp: false,
            nvidia: false,
            audio_layout: None,
            notices: vec![],
            hardware: serde_json::json!({}),
            decisions: vec![],
        };
        app.build_result = Some(Box::new(Build {
            schema: 1,
            ok: true,
            output: "Demo/Build".into(),
            plan,
            acpi: ferrite_core::acpi::AcpiResult {
                tables: vec![],
                generated: vec![],
                notices: vec![],
            },
            audit: Audit {
                root: "Demo/EFI".into(),
                issues: vec![],
                warnings: vec![],
                files: 0,
                config_sha256: String::new(),
                validator_passed: true,
                ready: true,
            },
            files: Default::default(),
            bios_guide: "Demo/BIOS.txt".into(),
        }));
        app.recovery_result = Some(Recovery {
            image: "Demo/Recovery/BaseSystem.dmg".into(),
            chunklist: "Demo/Recovery/BaseSystem.chunklist".into(),
            bytes: 888615275,
            chunks: 83,
            image_sha256: String::new(),
            chunklist_sha256: String::new(),
            board: String::new(),
            product: String::new(),
        });
    }
}

pub fn capture(app: &mut App, ctx: &egui::Context) {
    let Some(path) = std::env::var_os("FERRITE_SCREENSHOT_TO") else {
        return;
    };
    for event in ctx.input(|i| i.events.clone()) {
        if let egui::Event::Screenshot { image, .. } = event {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                PathBuf::from(&path),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .expect("Save UI screenshot");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
    }
    app.capture_frames = app.capture_frames.saturating_add(1);
    if app.capture_frames == 8 {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }
    ctx.request_repaint();
}
