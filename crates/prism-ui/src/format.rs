//! How Prism words and formats what it shows, so the view and its tests agree on every string.

/// What Prism shows in place of a reading it doesn't have.
pub const UNKNOWN: &str = "—";

/// A reading as shown: the number and its unit, drawn apart in large readouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readout {
    /// The number, or [`UNKNOWN`].
    pub value: String,
    pub unit: &'static str,
    known: bool,
}

impl Readout {
    pub fn celsius(value: Option<u32>) -> Self {
        Self::new(value, "°C")
    }

    pub fn percent(value: Option<u32>) -> Self {
        Self::new(value, "%")
    }

    pub fn watts(value: Option<u32>) -> Self {
        Self::new(value, "W")
    }

    fn new(value: Option<u32>, unit: &'static str) -> Self {
        Self {
            value: value.map_or_else(|| UNKNOWN.into(), |value| value.to_string()),
            unit,
            known: value.is_some(),
        }
    }

    pub fn is_known(&self) -> bool {
        self.known
    }

    /// The reading in running text, such as "71 °C", "96%" or "—".
    pub fn text(&self) -> String {
        match (self.known, self.unit) {
            (false, _) => UNKNOWN.into(),
            (true, "%") => format!("{}%", self.value),
            (true, unit) => format!("{} {unit}", self.value),
        }
    }

    /// The reading as a screen reader says it, with "unknown" for a missing reading.
    pub fn spoken(&self) -> String {
        if self.known {
            self.text()
        } else {
            "unknown".into()
        }
    }
}

/// A GPU's model name without the vendor and brand, such as "RTX 5090".
pub fn short_gpu_name(name: &str) -> &str {
    let name = name.strip_prefix("NVIDIA ").unwrap_or(name);
    name.strip_prefix("GeForce ").unwrap_or(name)
}

/// A PCI bus id without its domain when the domain is 0, such as "01:00.0".
pub fn short_bus_id(bus_id: &str) -> &str {
    match bus_id.split_once(':') {
        Some((domain, rest)) if domain.bytes().all(|digit| digit == b'0') => rest,
        _ => bus_id,
    }
}

#[cfg(test)]
mod tests {
    use super::{Readout, short_bus_id, short_gpu_name};

    #[test]
    fn readouts_show_their_value_and_unit() {
        assert_eq!(Readout::celsius(Some(71)).text(), "71 °C");
        assert_eq!(Readout::percent(Some(96)).text(), "96%");
        assert_eq!(Readout::watts(Some(312)).text(), "312 W");
        assert_eq!(Readout::celsius(Some(0)).text(), "0 °C");
    }

    #[test]
    fn an_unknown_reading_shows_as_unknown_never_zero() {
        let unknown = Readout::celsius(None);

        assert_eq!(unknown.value, "—");
        assert_eq!(unknown.text(), "—");
        assert_eq!(unknown.spoken(), "unknown");
        assert_eq!(Readout::celsius(Some(71)).spoken(), "71 °C");
    }

    #[test]
    fn gpu_names_drop_the_vendor_and_brand() {
        assert_eq!(short_gpu_name("NVIDIA GeForce RTX 5090"), "RTX 5090");
        assert_eq!(
            short_gpu_name("NVIDIA GeForce RTX 4060 Laptop GPU"),
            "RTX 4060 Laptop GPU"
        );
        assert_eq!(short_gpu_name("NVIDIA RTX A6000"), "RTX A6000");
        assert_eq!(short_gpu_name("Tesla T4"), "Tesla T4");
    }

    #[test]
    fn bus_ids_drop_an_empty_pci_domain() {
        assert_eq!(short_bus_id("00000000:01:00.0"), "01:00.0");
        assert_eq!(short_bus_id("0000:02:00.0"), "02:00.0");
        assert_eq!(short_bus_id("00000001:01:00.0"), "00000001:01:00.0");
    }
}
