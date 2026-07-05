use crate::{PowerError, PowerManagerBackend, PowerResult};
use powershift_core::PowerPlan;
use std::process::Command;

#[derive(Debug, Clone, Copy, Default)]
pub struct PowerCfgBackend;

impl PowerManagerBackend for PowerCfgBackend {
    fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
        let output = Command::new("powercfg").arg("/L").output()?;
        if !output.status.success() {
            return Err(PowerError::CommandFailed {
                command: "powercfg /L",
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        parse_powercfg_list(&String::from_utf8_lossy(&output.stdout))
            .map(|plans| plans.into_iter().map(|entry| entry.plan).collect())
    }

    fn active_plan(&self) -> PowerResult<PowerPlan> {
        let output = Command::new("powercfg").arg("/GETACTIVESCHEME").output()?;
        if !output.status.success() {
            return Err(PowerError::CommandFailed {
                command: "powercfg /GETACTIVESCHEME",
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        parse_powercfg_list(&String::from_utf8_lossy(&output.stdout))?
            .into_iter()
            .next()
            .map(|entry| entry.plan)
            .ok_or_else(|| PowerError::Parse("active power scheme was not found".to_string()))
    }

    fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
        let output = Command::new("powercfg").args(["/S", plan_id]).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(PowerError::CommandFailed {
                command: "powercfg /S",
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerCfgEntry {
    pub plan: PowerPlan,
    pub active: bool,
}

pub fn parse_powercfg_list(output: &str) -> PowerResult<Vec<PowerCfgEntry>> {
    let mut plans = Vec::new();

    for line in output.lines() {
        let Some(guid_start) = line.find(':') else {
            continue;
        };
        let rest = line[guid_start + 1..].trim();
        let Some((id, after_id)) = split_guid_prefix(rest) else {
            continue;
        };

        let active = after_id.contains('*');
        let name = extract_plan_name(after_id).unwrap_or_else(|| id.to_string());
        plans.push(PowerCfgEntry {
            plan: PowerPlan {
                id: normalize_guid(id),
                name,
            },
            active,
        });
    }

    if plans.is_empty() {
        Err(PowerError::Parse(
            "no power schemes were found in powercfg output".to_string(),
        ))
    } else {
        Ok(plans)
    }
}

fn split_guid_prefix(input: &str) -> Option<(&str, &str)> {
    let guid = input.get(0..36)?;
    if guid
        .chars()
        .enumerate()
        .all(|(index, ch)| matches!(index, 8 | 13 | 18 | 23) && ch == '-' || ch.is_ascii_hexdigit())
    {
        Some((guid, input.get(36..).unwrap_or_default()))
    } else {
        None
    }
}

fn extract_plan_name(input: &str) -> Option<String> {
    let start = input.find('(')?;
    let end = input[start + 1..].find(')')? + start + 1;
    let name = input[start + 1..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub fn normalize_guid(input: &str) -> String {
    input
        .trim()
        .trim_matches('{')
        .trim_matches('}')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_powercfg_list_output() {
        let output = r#"
Existing Power Schemes (* Active)
-----------------------------------
Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced) *
Power Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)
"#;

        let entries = parse_powercfg_list(output).expect("parse output");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].plan.name, "Balanced");
        assert_eq!(entries[0].plan.id, "381b4222-f694-41f0-9685-ff5bb260df2e");
        assert!(entries[0].active);
        assert!(!entries[1].active);
    }

    #[test]
    fn parses_spanish_powercfg_list_output() {
        let output = r#"
Combinaciones de energia existentes (* activo)
-----------------------------------
GUID de plan de energia: 381B4222-F694-41F0-9685-FF5BB260DF2E  (Equilibrado) *
GUID de plan de energia: a1841308-3541-4fab-bc81-f71556f20b4a  (Economizador)
"#;

        let entries = parse_powercfg_list(output).expect("parse output");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].plan.name, "Equilibrado");
        assert_eq!(entries[0].plan.id, "381b4222-f694-41f0-9685-ff5bb260df2e");
        assert!(entries[0].active);
    }

    #[test]
    fn parses_active_scheme_single_line_output() {
        let output = "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)\n";

        let entries = parse_powercfg_list(output).expect("parse output");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].plan.name, "Balanced");
    }

    #[test]
    fn returns_parse_error_when_no_guid_is_present() {
        let error = parse_powercfg_list("nothing useful").expect_err("expected parse error");

        assert!(matches!(error, PowerError::Parse(_)));
    }

    #[test]
    fn normalize_guid_strips_braces_and_lowercases() {
        assert_eq!(
            normalize_guid("{381B4222-F694-41F0-9685-FF5BB260DF2E}"),
            "381b4222-f694-41f0-9685-ff5bb260df2e"
        );
    }
}
