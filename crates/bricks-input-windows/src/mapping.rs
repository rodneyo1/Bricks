//! Explicit supported subset of PC/AT set-1 to USB keyboard-page usages.
//! Unsupported keys are errors; no virtual-key/text fallback guesses a physical key.
use crate::{InputError, Prefix, ScanTransition};
use bricks_protocol::PhysicalKey;

const NORMAL: &[(u16, u16)] = &[
    (0x01, 0x29),
    (0x02, 0x1e),
    (0x03, 0x1f),
    (0x04, 0x20),
    (0x05, 0x21),
    (0x06, 0x22),
    (0x07, 0x23),
    (0x08, 0x24),
    (0x09, 0x25),
    (0x0a, 0x26),
    (0x0b, 0x27),
    (0x0c, 0x2d),
    (0x0d, 0x2e),
    (0x0e, 0x2a),
    (0x0f, 0x2b),
    (0x10, 0x14),
    (0x11, 0x1a),
    (0x12, 0x08),
    (0x13, 0x15),
    (0x14, 0x17),
    (0x15, 0x1c),
    (0x16, 0x18),
    (0x17, 0x0c),
    (0x18, 0x12),
    (0x19, 0x13),
    (0x1a, 0x2f),
    (0x1b, 0x30),
    (0x1c, 0x28),
    (0x1d, 0xe0),
    (0x1e, 0x04),
    (0x1f, 0x16),
    (0x20, 0x07),
    (0x21, 0x09),
    (0x22, 0x0a),
    (0x23, 0x0b),
    (0x24, 0x0d),
    (0x25, 0x0e),
    (0x26, 0x0f),
    (0x27, 0x33),
    (0x28, 0x34),
    (0x29, 0x35),
    (0x2a, 0xe1),
    (0x2b, 0x31),
    (0x2c, 0x1d),
    (0x2d, 0x1b),
    (0x2e, 0x06),
    (0x2f, 0x19),
    (0x30, 0x05),
    (0x31, 0x11),
    (0x32, 0x10),
    (0x33, 0x36),
    (0x34, 0x37),
    (0x35, 0x38),
    (0x36, 0xe5),
    (0x37, 0x55),
    (0x38, 0xe2),
    (0x39, 0x2c),
    (0x3a, 0x39),
    (0x3b, 0x3a),
    (0x3c, 0x3b),
    (0x3d, 0x3c),
    (0x3e, 0x3d),
    (0x3f, 0x3e),
    (0x40, 0x3f),
    (0x41, 0x40),
    (0x42, 0x41),
    (0x43, 0x42),
    (0x44, 0x43),
    (0x45, 0x53),
    (0x46, 0x47),
    (0x47, 0x5f),
    (0x48, 0x60),
    (0x49, 0x61),
    (0x4a, 0x56),
    (0x4b, 0x5c),
    (0x4c, 0x5d),
    (0x4d, 0x5e),
    (0x4e, 0x57),
    (0x4f, 0x59),
    (0x50, 0x5a),
    (0x51, 0x5b),
    (0x52, 0x62),
    (0x53, 0x63),
    (0x56, 0x64),
    (0x57, 0x44),
    (0x58, 0x45),
];
const EXTENDED: &[(u16, u16)] = &[
    (0x1c, 0x58),
    (0x1d, 0xe4),
    (0x35, 0x54),
    (0x37, 0x46),
    (0x38, 0xe6),
    (0x47, 0x4a),
    (0x48, 0x52),
    (0x49, 0x4b),
    (0x4b, 0x50),
    (0x4d, 0x4f),
    (0x4f, 0x4d),
    (0x50, 0x51),
    (0x51, 0x4e),
    (0x52, 0x49),
    (0x53, 0x4c),
    (0x5b, 0xe3),
    (0x5c, 0xe7),
    (0x5d, 0x65),
];

/// Aggregate diagnostic categories only: never returns a key code or typed text.
pub fn unsupported_reason(make_code: u16, flags: u16) -> Option<&'static str> {
    if make_code == 0 {
        return Some("missing_scan_code");
    }
    if make_code == 0xff {
        return Some("keyboard_overrun_marker");
    }
    if make_code > 0x7f {
        return Some("scan_code_out_of_supported_range");
    }
    if flags & !7 != 0 {
        return Some("unrecognized_raw_flags");
    }
    if flags & 6 == 6 {
        return Some("conflicting_prefix_flags");
    }
    let event = crate::normalize_keyboard(make_code, flags).ok()?;
    if event.prefix == Prefix::E1 {
        return Some("e1_sequence_not_supported");
    }
    physical(event)
        .err()
        .map(|_| "scan_code_not_in_mapping_table")
}
pub fn physical(event: ScanTransition) -> Result<PhysicalKey, InputError> {
    let table = match event.prefix {
        Prefix::None => NORMAL,
        Prefix::E0 => EXTENDED,
        Prefix::E1 => return Err(InputError::UnsupportedE1),
    };
    table
        .iter()
        .find(|&&(scan, _)| scan == event.make_code)
        .map(|&(_, usage)| PhysicalKey {
            usage_page: 7,
            usage,
        })
        .ok_or(InputError::InvalidRecord)
}
pub fn scancode(key: PhysicalKey, down: bool) -> Result<ScanTransition, InputError> {
    if key.usage_page != 7 {
        return Err(InputError::InvalidRecord);
    }
    for (prefix, table) in [(Prefix::None, NORMAL), (Prefix::E0, EXTENDED)] {
        if let Some(&(make_code, _)) = table.iter().find(|&&(_, usage)| usage == key.usage) {
            return Ok(ScanTransition {
                make_code,
                prefix,
                down,
            });
        }
    }
    Err(InputError::InvalidRecord)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_reasons_distinguish_missing_mapping_from_invalid_records() {
        assert_eq!(unsupported_reason(0, 0), Some("missing_scan_code"));
        assert_eq!(unsupported_reason(0xff, 0), Some("keyboard_overrun_marker"));
        assert_eq!(unsupported_reason(0x1e, 8), Some("unrecognized_raw_flags"));
        assert_eq!(
            unsupported_reason(0x1e, 6),
            Some("conflicting_prefix_flags")
        );
        assert_eq!(
            unsupported_reason(0x45, 4),
            Some("e1_sequence_not_supported")
        );
        assert_eq!(
            unsupported_reason(0x7f, 0),
            Some("scan_code_not_in_mapping_table")
        );
        assert_eq!(unsupported_reason(0x1e, 0), None);
        assert_eq!(unsupported_reason(0x1e, 1), None);
    }
    #[test]
    fn all_supported_keys_are_unique_and_roundtrip_both_transitions() {
        let mut usages = std::collections::BTreeSet::new();
        for (prefix, table) in [(Prefix::None, NORMAL), (Prefix::E0, EXTENDED)] {
            for &(make_code, usage) in table {
                assert!(usages.insert(usage), "ambiguous HID usage {usage}");
                for down in [true, false] {
                    let event = ScanTransition {
                        make_code,
                        prefix,
                        down,
                    };
                    assert_eq!(scancode(physical(event).unwrap(), down).unwrap(), event);
                }
            }
        }
    }
    #[test]
    fn distinguishes_modifiers_and_rejects_unmapped_inputs() {
        assert_eq!(
            physical(crate::normalize_keyboard(0x1d, 0).unwrap())
                .unwrap()
                .usage,
            0xe0
        );
        assert_eq!(
            physical(crate::normalize_keyboard(0x1d, 2).unwrap())
                .unwrap()
                .usage,
            0xe4
        );
        assert!(
            scancode(
                PhysicalKey {
                    usage_page: 7,
                    usage: 0x48
                },
                true
            )
            .is_err()
        ); // Pause
        assert!(
            scancode(
                PhysicalKey {
                    usage_page: 12,
                    usage: 4
                },
                true
            )
            .is_err()
        );
    }
}
