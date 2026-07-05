use powershift_core::PowerPlan;
use powershift_windows::{PowerManager, PowerManagerBackend};

pub fn list_power_plans_with<B: PowerManagerBackend>(
    backend: &B,
) -> Result<Vec<PowerPlan>, String> {
    backend.list_plans().map_err(|error| error.to_string())
}

pub fn active_power_plan_with<B: PowerManagerBackend>(backend: &B) -> Result<PowerPlan, String> {
    backend.active_plan().map_err(|error| error.to_string())
}

pub fn set_active_power_plan_with<B: PowerManagerBackend>(
    backend: &B,
    plan_id: &str,
) -> Result<(), String> {
    backend
        .set_active_plan(plan_id)
        .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_power_plans() -> Result<Vec<PowerPlan>, String> {
    list_power_plans_with(&PowerManager::new())
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_active_power_plan() -> Result<PowerPlan, String> {
    active_power_plan_with(&PowerManager::new())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_active_power_plan(plan_id: String) -> Result<(), String> {
    set_active_power_plan_with(&PowerManager::new(), &plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_windows::{PowerError, PowerResult};
    use std::cell::RefCell;

    struct FakePowerBackend {
        plans: Vec<PowerPlan>,
        active: PowerPlan,
        set_calls: RefCell<Vec<String>>,
        fail_list: bool,
    }

    impl FakePowerBackend {
        fn working() -> Self {
            Self {
                plans: vec![
                    PowerPlan {
                        id: "balanced".to_string(),
                        name: "Equilibrado".to_string(),
                    },
                    PowerPlan {
                        id: "high".to_string(),
                        name: "Alto rendimiento".to_string(),
                    },
                ],
                active: PowerPlan {
                    id: "balanced".to_string(),
                    name: "Equilibrado".to_string(),
                },
                set_calls: RefCell::new(Vec::new()),
                fail_list: false,
            }
        }
    }

    impl PowerManagerBackend for FakePowerBackend {
        fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
            if self.fail_list {
                Err(PowerError::Parse("boom".to_string()))
            } else {
                Ok(self.plans.clone())
            }
        }

        fn active_plan(&self) -> PowerResult<PowerPlan> {
            Ok(self.active.clone())
        }

        fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
            self.set_calls.borrow_mut().push(plan_id.to_string());
            Ok(())
        }
    }

    #[test]
    fn list_power_plans_returns_backend_plans() {
        let backend = FakePowerBackend::working();

        let plans = list_power_plans_with(&backend).expect("list plans");

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].id, "balanced");
    }

    #[test]
    fn list_power_plans_converts_backend_errors_to_string() {
        let mut backend = FakePowerBackend::working();
        backend.fail_list = true;

        let error = list_power_plans_with(&backend).expect_err("expected error");

        assert!(error.contains("boom"));
    }

    #[test]
    fn active_power_plan_returns_backend_active_plan() {
        let backend = FakePowerBackend::working();

        let active = active_power_plan_with(&backend).expect("active plan");

        assert_eq!(active.id, "balanced");
    }

    #[test]
    fn set_active_power_plan_passes_plan_id_to_backend() {
        let backend = FakePowerBackend::working();

        set_active_power_plan_with(&backend, "high").expect("set plan");

        assert_eq!(backend.set_calls.borrow().as_slice(), &["high".to_string()]);
    }
}
