use crate::{PowerCfgBackend, PowerResult};
use powershift_core::PowerPlan;

pub trait PowerManagerBackend {
    fn list_plans(&self) -> PowerResult<Vec<PowerPlan>>;
    fn active_plan(&self) -> PowerResult<PowerPlan>;
    fn set_active_plan(&self, plan_id: &str) -> PowerResult<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PowerManager;

impl PowerManager {
    pub fn new() -> Self {
        Self
    }
}

impl PowerManagerBackend for PowerManager {
    fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
        #[cfg(windows)]
        {
            crate::native::NativePowerBackend
                .list_plans()
                .or_else(|_| PowerCfgBackend.list_plans())
        }

        #[cfg(not(windows))]
        {
            PowerCfgBackend::default().list_plans()
        }
    }

    fn active_plan(&self) -> PowerResult<PowerPlan> {
        #[cfg(windows)]
        {
            crate::native::NativePowerBackend
                .active_plan()
                .or_else(|_| PowerCfgBackend.active_plan())
        }

        #[cfg(not(windows))]
        {
            PowerCfgBackend::default().active_plan()
        }
    }

    fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
        #[cfg(windows)]
        {
            crate::native::NativePowerBackend
                .set_active_plan(plan_id)
                .or_else(|_| PowerCfgBackend.set_active_plan(plan_id))
        }

        #[cfg(not(windows))]
        {
            PowerCfgBackend::default().set_active_plan(plan_id)
        }
    }
}
