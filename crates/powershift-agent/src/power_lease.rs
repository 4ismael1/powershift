use crate::AgentRuntimeState;
use powershift_core::{write_file_atomically, AppConfig, Profile, RestoreBehavior};
use powershift_windows::PowerManagerBackend;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const POWER_CONTROL_LEASE_VERSION: u8 = 1;
const MAX_POWER_CONTROL_LEASE_BYTES: u64 = 16 * 1024;
const MAX_LEASE_TEXT_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PowerControlLease {
    version: u8,
    pub(crate) previous_plan_id: String,
    pub(crate) managed_plan_id: String,
    pub(crate) profile_id: String,
    pub(crate) profile_name: String,
    pub(crate) restore_plan_id: Option<String>,
    pub(crate) close_delay_seconds: u32,
}

impl PowerControlLease {
    pub(crate) fn for_profile(
        profile: &Profile,
        previous_plan_id: impl Into<String>,
        managed_plan_id: impl Into<String>,
    ) -> Self {
        let previous_plan_id = previous_plan_id.into();
        let restore_plan_id = match profile.power.on_close_behavior {
            RestoreBehavior::PreviousPlan => Some(previous_plan_id.clone()),
            RestoreBehavior::SpecificPlan => profile.power.on_close_plan_id.clone(),
            RestoreBehavior::DoNothing => None,
        };

        Self {
            version: POWER_CONTROL_LEASE_VERSION,
            previous_plan_id,
            managed_plan_id: managed_plan_id.into(),
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            restore_plan_id,
            close_delay_seconds: profile.power.close_delay_seconds,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != POWER_CONTROL_LEASE_VERSION {
            return Err("Version de journal de energia no compatible.".to_string());
        }
        for (label, value) in [
            ("plan anterior", self.previous_plan_id.as_str()),
            ("plan administrado", self.managed_plan_id.as_str()),
            ("perfil", self.profile_id.as_str()),
            ("nombre del perfil", self.profile_name.as_str()),
        ] {
            if value.trim().is_empty() || value.len() > MAX_LEASE_TEXT_BYTES {
                return Err(format!("Campo invalido en journal de energia: {label}."));
            }
        }
        if self
            .restore_plan_id
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > MAX_LEASE_TEXT_BYTES)
        {
            return Err("Plan de restauracion invalido en journal de energia.".to_string());
        }
        if self.close_delay_seconds > 3600 {
            return Err("Retardo invalido en journal de energia.".to_string());
        }
        Ok(())
    }
}

pub(crate) trait PowerLeaseJournal {
    fn current_lease(&self) -> Option<&PowerControlLease>;
    fn record(&mut self, lease: PowerControlLease) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub(crate) struct NoopPowerLeaseJournal;

impl PowerLeaseJournal for NoopPowerLeaseJournal {
    fn current_lease(&self) -> Option<&PowerControlLease> {
        None
    }

    fn record(&mut self, _lease: PowerControlLease) -> Result<(), String> {
        Ok(())
    }

    fn clear(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PowerLeaseStore {
    path: PathBuf,
    current: Option<PowerControlLease>,
    warning: Option<String>,
}

impl PowerLeaseStore {
    pub(crate) fn open(path: PathBuf) -> Result<Self, String> {
        let mut store = Self {
            path,
            current: None,
            warning: None,
        };
        let metadata = match std::fs::metadata(&store.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(error) => return Err(error.to_string()),
        };

        if metadata.len() > MAX_POWER_CONTROL_LEASE_BYTES {
            store.quarantine("El journal de energia excede el tamano permitido.");
            return Ok(store);
        }

        let contents = match std::fs::read_to_string(&store.path) {
            Ok(contents) => contents,
            Err(error) => {
                store.quarantine(&format!("No se pudo leer el journal de energia: {error}"));
                return Ok(store);
            }
        };
        match serde_json::from_str::<PowerControlLease>(&contents)
            .map_err(|error| error.to_string())
            .and_then(|lease| lease.validate().map(|()| lease))
        {
            Ok(lease) => store.current = Some(lease),
            Err(error) => store.quarantine(&format!("Journal de energia invalido: {error}")),
        }
        Ok(store)
    }

    pub(crate) fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    fn quarantine(&mut self, warning: &str) {
        let quarantine_path = corrupt_lease_path(&self.path);
        let _ = std::fs::rename(&self.path, quarantine_path);
        self.warning = Some(warning.to_string());
        self.current = None;
    }
}

impl PowerLeaseJournal for PowerLeaseStore {
    fn current_lease(&self) -> Option<&PowerControlLease> {
        self.current.as_ref()
    }

    fn record(&mut self, lease: PowerControlLease) -> Result<(), String> {
        lease.validate()?;
        if self.current.as_ref() == Some(&lease) {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&lease).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_POWER_CONTROL_LEASE_BYTES {
            return Err("El journal de energia excede el tamano permitido.".to_string());
        }
        write_file_atomically(&self.path, &bytes).map_err(|error| error.to_string())?;
        self.current = Some(lease);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        self.current = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerLeaseRecovery {
    None,
    Resumed,
    ReleasedAfterExternalChange,
    RestoredOrReleased,
}

pub(crate) fn recover_power_control_lease<W>(
    config: &AppConfig,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    journal: &mut PowerLeaseStore,
) -> Result<PowerLeaseRecovery, String>
where
    W: PowerManagerBackend,
{
    let Some(lease) = journal.current_lease().cloned() else {
        return Ok(PowerLeaseRecovery::None);
    };
    let active_plan = power_backend
        .active_plan()
        .map_err(|error| error.to_string())?;

    // If another actor changed the plan after PowerShift took control, the
    // user's current choice wins and the stale lease must not overwrite it.
    if active_plan.id != lease.managed_plan_id {
        journal.clear()?;
        return Ok(PowerLeaseRecovery::ReleasedAfterExternalChange);
    }

    if config
        .profiles
        .iter()
        .any(|profile| profile.id == lease.profile_id)
    {
        state.previous_plan_id = Some(lease.previous_plan_id);
        state.winning_profile_id = Some(lease.profile_id);
        return Ok(PowerLeaseRecovery::Resumed);
    }

    if let Some(restore_plan_id) = lease.restore_plan_id {
        if active_plan.id != restore_plan_id {
            power_backend
                .set_active_plan(&restore_plan_id)
                .map_err(|error| error.to_string())?;
        }
    }
    journal.clear()?;
    Ok(PowerLeaseRecovery::RestoredOrReleased)
}

pub(crate) fn release_power_control<W, J>(power_backend: &W, journal: &mut J) -> Result<(), String>
where
    W: PowerManagerBackend,
    J: PowerLeaseJournal,
{
    let Some(lease) = journal.current_lease().cloned() else {
        return Ok(());
    };
    let active_plan = power_backend
        .active_plan()
        .map_err(|error| error.to_string())?;
    if active_plan.id == lease.managed_plan_id {
        if let Some(restore_plan_id) = lease.restore_plan_id {
            if active_plan.id != restore_plan_id {
                power_backend
                    .set_active_plan(&restore_plan_id)
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    journal.clear()
}

fn corrupt_lease_path(path: &Path) -> PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("power-control-lease.json");
    path.with_file_name(format!("{file_name}.corrupt-{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_core::{PowerPlan, Profile};
    use powershift_windows::PowerResult;
    use std::cell::RefCell;

    struct FakePowerBackend {
        active: RefCell<PowerPlan>,
        set_calls: RefCell<Vec<String>>,
    }

    impl FakePowerBackend {
        fn new(active: &str) -> Self {
            Self {
                active: RefCell::new(PowerPlan {
                    id: active.to_string(),
                    name: active.to_string(),
                }),
                set_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl PowerManagerBackend for FakePowerBackend {
        fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
            Ok(vec![self.active.borrow().clone()])
        }

        fn active_plan(&self) -> PowerResult<PowerPlan> {
            Ok(self.active.borrow().clone())
        }

        fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
            self.set_calls.borrow_mut().push(plan_id.to_string());
            self.active.borrow_mut().id = plan_id.to_string();
            Ok(())
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "powershift-power-lease-{name}-{}.json",
            std::process::id()
        ))
    }

    fn lease() -> PowerControlLease {
        PowerControlLease::for_profile(
            &Profile::new("game", "Game", "game.exe", "high"),
            "balanced",
            "high",
        )
    }

    #[test]
    fn store_roundtrips_and_clears_a_valid_lease() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");

        store.record(lease()).expect("record lease");
        let reopened = PowerLeaseStore::open(path.clone()).expect("reopen store");
        assert_eq!(reopened.current_lease(), Some(&lease()));

        store.clear().expect("clear lease");
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_lease_is_quarantined_without_blocking_agent_startup() {
        let path = temp_path("corrupt");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"{invalid").expect("seed corrupt lease");

        let mut store = PowerLeaseStore::open(path.clone()).expect("recover store");

        assert!(store.current_lease().is_none());
        assert!(store.take_warning().is_some());
        assert!(!path.exists());
        if let Some(parent) = path.parent() {
            let prefix = format!("{}.corrupt-", path.file_name().unwrap().to_string_lossy());
            for entry in parent.read_dir().expect("read temp dir").flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    #[test]
    fn startup_resumes_original_context_when_managed_profile_still_exists() {
        let path = temp_path("resume");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");
        store.record(lease()).expect("record lease");
        let config = AppConfig {
            profiles: vec![Profile::new("game", "Game", "game.exe", "high")],
            ..AppConfig::default()
        };
        let mut state = AgentRuntimeState::default();

        let recovery = recover_power_control_lease(
            &config,
            &FakePowerBackend::new("high"),
            &mut state,
            &mut store,
        )
        .expect("recover lease");

        assert_eq!(recovery, PowerLeaseRecovery::Resumed);
        assert_eq!(state.previous_plan_id.as_deref(), Some("balanced"));
        assert_eq!(state.winning_profile_id.as_deref(), Some("game"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn removed_profile_restores_previous_plan_during_startup_recovery() {
        let path = temp_path("removed-profile");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");
        store.record(lease()).expect("record lease");
        let power = FakePowerBackend::new("high");

        let recovery = recover_power_control_lease(
            &AppConfig::default(),
            &power,
            &mut AgentRuntimeState::default(),
            &mut store,
        )
        .expect("recover lease");

        assert_eq!(recovery, PowerLeaseRecovery::RestoredOrReleased);
        assert_eq!(power.set_calls.borrow().as_slice(), &["balanced"]);
        assert!(!path.exists());
    }

    #[test]
    fn external_plan_change_wins_over_stale_recovery_lease() {
        let path = temp_path("external-change");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");
        store.record(lease()).expect("record lease");
        let power = FakePowerBackend::new("power-saver");

        let recovery = recover_power_control_lease(
            &AppConfig::default(),
            &power,
            &mut AgentRuntimeState::default(),
            &mut store,
        )
        .expect("recover lease");

        assert_eq!(recovery, PowerLeaseRecovery::ReleasedAfterExternalChange);
        assert!(power.set_calls.borrow().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn release_does_not_overwrite_a_manual_plan_change() {
        let path = temp_path("manual-release");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");
        store.record(lease()).expect("record lease");
        let power = FakePowerBackend::new("power-saver");

        release_power_control(&power, &mut store).expect("release control");

        assert!(power.set_calls.borrow().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn release_restores_the_captured_plan_while_powershift_still_controls_power() {
        let path = temp_path("managed-release");
        let _ = std::fs::remove_file(&path);
        let mut store = PowerLeaseStore::open(path.clone()).expect("open store");
        store.record(lease()).expect("record lease");
        let power = FakePowerBackend::new("high");

        release_power_control(&power, &mut store).expect("release control");

        assert_eq!(power.set_calls.borrow().as_slice(), &["balanced"]);
        assert!(!path.exists());
    }
}
