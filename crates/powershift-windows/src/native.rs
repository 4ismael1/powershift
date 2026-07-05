use crate::{PowerError, PowerManagerBackend, PowerResult};
use powershift_core::PowerPlan;
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HLOCAL};
use windows::Win32::System::Power::{
    PowerEnumerate, PowerGetActiveScheme, PowerReadFriendlyName, PowerSetActiveScheme,
    ACCESS_SCHEME,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct NativePowerBackend;

impl PowerManagerBackend for NativePowerBackend {
    fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
        enumerate_power_schemes()
    }

    fn active_plan(&self) -> PowerResult<PowerPlan> {
        let id = active_scheme_id()?;
        let name = friendly_name_for_scheme(&parse_guid(&id)?)?;
        Ok(PowerPlan { id, name })
    }

    fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
        let guid = parse_guid(plan_id)?;
        let result = unsafe { PowerSetActiveScheme(None, Some(&guid)) };
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(PowerError::WindowsApi {
                function: "PowerSetActiveScheme",
                code: result.0,
            })
        }
    }
}

fn enumerate_power_schemes() -> PowerResult<Vec<PowerPlan>> {
    let mut plans = Vec::new();
    let mut index = 0;

    loop {
        let mut size = std::mem::size_of::<GUID>() as u32;
        let mut guid = GUID::zeroed();
        let result = unsafe {
            PowerEnumerate(
                None,
                None,
                None,
                ACCESS_SCHEME,
                index,
                Some((&mut guid as *mut GUID).cast::<u8>()),
                &mut size,
            )
        };

        if result == ERROR_NO_MORE_ITEMS {
            break;
        }

        if result != ERROR_SUCCESS {
            return Err(PowerError::WindowsApi {
                function: "PowerEnumerate",
                code: result.0,
            });
        }

        let id = format_guid(&guid);
        let name = friendly_name_for_scheme(&guid).unwrap_or_else(|_| id.clone());
        plans.push(PowerPlan { id, name });
        index += 1;
    }

    if plans.is_empty() {
        Err(PowerError::Parse(
            "PowerEnumerate returned no power schemes".to_string(),
        ))
    } else {
        Ok(plans)
    }
}

fn active_scheme_id() -> PowerResult<String> {
    let mut guid_ptr: *mut GUID = std::ptr::null_mut();
    let result = unsafe { PowerGetActiveScheme(None, &mut guid_ptr) };
    if result != ERROR_SUCCESS {
        return Err(PowerError::WindowsApi {
            function: "PowerGetActiveScheme",
            code: result.0,
        });
    }

    if guid_ptr.is_null() {
        return Err(PowerError::WindowsApi {
            function: "PowerGetActiveScheme",
            code: result.0,
        });
    }

    let id = unsafe {
        let guid = *guid_ptr;
        let _ = LocalFree(Some(HLOCAL(guid_ptr.cast())));
        format_guid(&guid)
    };
    Ok(id)
}

fn friendly_name_for_scheme(guid: &GUID) -> PowerResult<String> {
    let mut size = 0u32;
    let probe = unsafe { PowerReadFriendlyName(None, Some(guid), None, None, None, &mut size) };
    if probe != ERROR_SUCCESS && size == 0 {
        return Err(PowerError::WindowsApi {
            function: "PowerReadFriendlyName",
            code: probe.0,
        });
    }

    let mut buffer = vec![0u8; size as usize];
    let result = unsafe {
        PowerReadFriendlyName(
            None,
            Some(guid),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            &mut size,
        )
    };

    if result != ERROR_SUCCESS {
        return Err(PowerError::WindowsApi {
            function: "PowerReadFriendlyName",
            code: result.0,
        });
    }

    utf16_bytes_to_string(&buffer)
}

fn utf16_bytes_to_string(buffer: &[u8]) -> PowerResult<String> {
    if !buffer.len().is_multiple_of(2) {
        return Err(PowerError::Parse(
            "friendly name buffer has odd byte length".to_string(),
        ));
    }

    let words: Vec<u16> = buffer
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|word| *word != 0)
        .collect();

    String::from_utf16(&words)
        .map_err(|_| PowerError::Parse("friendly name is not valid UTF-16".to_string()))
}

fn parse_guid(input: &str) -> PowerResult<GUID> {
    let normalized = input
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase();
    let mut compact = String::with_capacity(32);

    for (index, ch) in normalized.chars().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if ch != '-' {
                return Err(PowerError::InvalidGuid(input.to_string()));
            }
        } else if ch.is_ascii_hexdigit() {
            compact.push(ch);
        } else {
            return Err(PowerError::InvalidGuid(input.to_string()));
        }
    }

    if compact.len() != 32 {
        return Err(PowerError::InvalidGuid(input.to_string()));
    }

    let value = u128::from_str_radix(&compact, 16)
        .map_err(|_| PowerError::InvalidGuid(input.to_string()))?;
    Ok(GUID::from_u128(value))
}

fn format_guid(guid: &GUID) -> String {
    let value = guid.to_u128();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (value >> 96) as u32,
        ((value >> 80) & 0xffff) as u16,
        ((value >> 64) & 0xffff) as u16,
        ((value >> 48) & 0xffff) as u16,
        value & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

    #[test]
    fn parses_and_formats_guid_roundtrip() {
        let guid = parse_guid(BALANCED).expect("parse guid");

        assert_eq!(format_guid(&guid), BALANCED);
    }

    #[test]
    fn parses_guid_with_braces() {
        let guid = parse_guid("{381B4222-F694-41F0-9685-FF5BB260DF2E}").expect("parse guid");

        assert_eq!(format_guid(&guid), BALANCED);
    }

    #[test]
    fn rejects_guid_with_missing_hyphens() {
        let error = parse_guid("381b4222f69441f09685ff5bb260df2e").expect_err("invalid guid");

        assert!(matches!(error, PowerError::InvalidGuid(_)));
    }

    #[test]
    fn rejects_guid_with_invalid_character() {
        let error = parse_guid("381b4222-f694-41f0-9685-ff5bb260df2x").expect_err("invalid guid");

        assert!(matches!(error, PowerError::InvalidGuid(_)));
    }

    #[test]
    fn decodes_utf16_friendly_name_bytes() {
        let bytes = [0x45, 0x00, 0x71, 0x00, 0x75, 0x00, 0x69, 0x00, 0x00, 0x00];

        assert_eq!(utf16_bytes_to_string(&bytes).expect("decode"), "Equi");
    }

    #[test]
    fn rejects_odd_utf16_buffer() {
        let error = utf16_bytes_to_string(&[0x45]).expect_err("expected error");

        assert!(matches!(error, PowerError::Parse(_)));
    }

    #[test]
    #[ignore = "changes and restores the real Windows power plan; run intentionally only"]
    fn integration_lists_active_and_restores_current_plan() {
        let backend = NativePowerBackend;
        let original = backend.active_plan().expect("active plan");
        let plans = backend.list_plans().expect("list plans");
        assert!(plans.iter().any(|plan| plan.id == original.id));

        backend
            .set_active_plan(&original.id)
            .expect("restore original plan");
    }
}
