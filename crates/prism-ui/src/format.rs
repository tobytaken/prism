//! How Prism words and formats readings and states, so the view and its tests agree on them.

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

    pub fn megahertz(value: Option<u32>) -> Self {
        Self::new(value, "MHz")
    }

    /// A clock offset, signed so a raise reads "+150 MHz" and a cut "-500 MHz".
    pub fn offset_megahertz(value: Option<i32>) -> Self {
        Self::shown(value.map(signed), "MHz")
    }

    fn new(value: Option<u32>, unit: &'static str) -> Self {
        Self::shown(value.map(|value| value.to_string()), unit)
    }

    fn shown(value: Option<String>, unit: &'static str) -> Self {
        Self {
            known: value.is_some(),
            value: value.unwrap_or_else(|| UNKNOWN.into()),
            unit,
        }
    }

    pub fn is_known(&self) -> bool {
        self.known
    }

    /// The reading in running text, such as "71 °C", "96%" or "—".
    pub fn text(&self) -> String {
        if self.known {
            format!("{}{}", self.value, self.suffix())
        } else {
            UNKNOWN.into()
        }
    }

    /// What follows the number, such as " °C" or "%": a percent sign follows it without a space.
    /// Nothing follows an unknown reading.
    pub fn suffix(&self) -> String {
        match (self.known, self.unit) {
            (false, _) => String::new(),
            (true, "%") => "%".into(),
            (true, unit) => format!(" {unit}"),
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

/// A pending value as shown next to its control, such as "→ 450 W" or "→ default".
pub fn pending(value: &str) -> String {
    format!("→ {value}")
}

/// A signed number with its sign, such as "+150", "0" or "-500".
pub fn signed(value: i32) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
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
        assert_eq!(Readout::megahertz(Some(2_790)).text(), "2790 MHz");
    }

    #[test]
    fn a_unit_follows_its_number_with_a_space_except_a_percent_sign() {
        assert_eq!(Readout::celsius(Some(71)).suffix(), " °C");
        assert_eq!(Readout::percent(Some(96)).suffix(), "%");
        assert_eq!(Readout::percent(None).suffix(), "");
    }

    #[test]
    fn offsets_show_their_sign() {
        assert_eq!(Readout::offset_megahertz(Some(150)).text(), "+150 MHz");
        assert_eq!(Readout::offset_megahertz(Some(0)).text(), "0 MHz");
        assert_eq!(Readout::offset_megahertz(Some(-500)).text(), "-500 MHz");
        assert_eq!(Readout::offset_megahertz(None).text(), "—");
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
