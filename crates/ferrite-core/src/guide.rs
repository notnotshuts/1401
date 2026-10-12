//! The same localized, step-by-step guide is used by the desktop and CLI.
use crate::{
    i18n::{Language, format_for, text_for},
    planner::Plan,
};
pub const STEPS: [(&str, &str); 6] = [
    ("guide.step1.title", "guide.step1.body"),
    ("guide.step2.title", "guide.step2.body"),
    ("guide.step3.title", "guide.step3.body"),
    ("guide.step4.title", "guide.step4.body"),
    ("guide.step5.title", "guide.step5.body"),
    ("guide.step6.title", "guide.step6.body"),
];
pub const BIOS: [&str; 8] = [
    "guide.bios.uefi",
    "guide.bios.secure",
    "guide.bios.fast",
    "guide.bios.storage",
    "guide.bios.xhci",
    "guide.bios.4g",
    "guide.bios.bar",
    "guide.bios.intel",
];
pub const CHECKS: [&str; 6] = [
    "guide.test.network",
    "guide.test.audio",
    "guide.test.usb",
    "guide.test.display",
    "guide.test.sleep",
    "guide.test.boot",
];
pub fn render(plan: Option<&Plan>, language: Language) -> String {
    let t = |key| text_for(language, key);
    let mut out = format!(
        "# Ferrite — {}\n\n{}\n\n",
        t("guide.title"),
        t("guide.subtitle")
    );
    if let Some(plan) = plan {
        out.push_str(&format_for(
            language,
            "guide.context",
            &[
                ("0", crate::compatibility::version_name(plan.target).into()),
                ("1", plan.smbios.clone()),
            ],
        ));
        out.push_str("\n\n");
    }
    for (i, (title, body)) in STEPS.iter().enumerate() {
        out.push_str(&format!("## {}\n\n{}\n\n", t(title), t(body)));
        if i == 1 {
            out.push_str(t("guide.bios.check_hint"));
            out.push_str("\n\n");
            for key in BIOS {
                if key == "guide.bios.intel"
                    && plan.is_some_and(|p| {
                        p.hardware["CPU"]["Manufacturer"]
                            .as_str()
                            .is_some_and(|s| s.contains("AMD"))
                    })
                {
                    continue;
                }
                out.push_str(&format!("- [ ] {}\n", t(key)));
            }
            out.push('\n');
        }
    }
    if plan.is_some_and(|p| p.nvidia) {
        out.push_str(t("guide.nvidia"));
        out.push_str("\n\n");
    }
    out.push_str(&format!("## {}\n\n", t("guide.checklist")));
    for key in CHECKS {
        out.push_str(&format!("- [ ] {}\n", t(key)));
    }
    out.push_str(&format!(
        "\n## {}\n\n{}\n",
        t("guide.troubleshoot.title"),
        t("guide.troubleshoot.body")
    ));
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guide_available_in_all_languages() {
        for language in Language::ALL {
            let text = render(None, language);
            assert_eq!(text.matches("## ").count(), 8);
            assert!(!text.contains("guide."));
            assert!(!text.contains("OpCore-Simplify"));
            assert!(text.contains("USB"));
        }
    }
}
