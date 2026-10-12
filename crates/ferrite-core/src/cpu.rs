use serde_json::Value;
use std::sync::LazyLock;
static IDENTIFIERS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/cpu-identifiers.json")).expect("CPU identifiers")
});
pub fn codename(name: &str, identifier: &str) -> String {
    if identifier == "Family 6 Model 142 Stepping 10"
        && (name.contains("0U") || name.contains("7U"))
        && !name.contains("82")
    {
        return "Kaby Lake-R".into();
    }
    for row in IDENTIFIERS
        .as_array()
        .expect("identifier array")
        .iter()
        .rev()
    {
        let Some(id) = row[1].as_str() else {
            continue;
        };
        if identifier.contains(id)
            && row
                .get(2)
                .and_then(Value::as_str)
                .is_none_or(|hint| name.contains(hint))
        {
            return row[0].as_str().unwrap_or("Unknown").into();
        }
    }
    "Unknown".into()
}
pub fn pci_segment(name: &str) -> Option<String> {
    let r = regex::Regex::new(r"^[0-9A-Fa-f]{4}:[0-9A-Fa-f]{2}:([0-9A-Fa-f]{2})\.([0-7])$").ok()?;
    let c = r.captures(name)?;
    Some(format!(
        "/Pci(0x{:X},0x{})",
        u8::from_str_radix(&c[1], 16).ok()?,
        &c[2]
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_user_tiger_lake() {
        assert_eq!(
            codename(
                "11th Gen Intel Core i7-11800H",
                "Family 6 Model 141 Stepping 1"
            ),
            "Tiger Lake-H"
        );
    }
    #[test]
    fn pci_path_segment() {
        assert_eq!(
            pci_segment("0000:03:00.1").as_deref(),
            Some("/Pci(0x0,0x1)")
        );
        assert!(pci_segment("../etc").is_none());
    }
}
