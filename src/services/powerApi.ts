export interface PowerPlan {
  id: string;
  name: string;
}

export type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export const FALLBACK_POWER_PLAN_NAMES = [
  'Maximo rendimiento',
  'Alto rendimiento',
  'Equilibrado',
  'Ahorro de energia',
] as const;

export async function getPowerPlans(invokeFn: InvokeFn): Promise<PowerPlan[]> {
  return invokeFn<PowerPlan[]>('get_power_plans');
}

export async function getActivePowerPlan(invokeFn: InvokeFn): Promise<PowerPlan> {
  return invokeFn<PowerPlan>('get_active_power_plan');
}

export async function setActivePowerPlan(invokeFn: InvokeFn, planId: string): Promise<void> {
  return invokeFn<void>('set_active_power_plan', { plan_id: planId });
}

export function toPowerPlanOptions(
  plans: PowerPlan[],
  fallbackNames: readonly string[] = FALLBACK_POWER_PLAN_NAMES,
): PowerPlan[] {
  if (plans.length > 0) return uniquePlans(plans);
  return fallbackNames.map((name) => ({ id: name, name }));
}

export function resolvePlanId(plans: PowerPlan[], selectedValue: string): string | undefined {
  const normalized = normalize(selectedValue);
  if (!normalized) return undefined;

  const byId = plans.find((plan) => normalize(plan.id) === normalized);
  if (byId) return byId.id;

  const byName = plans.find((plan) => normalize(plan.name) === normalized);
  if (byName) return byName.id;

  return undefined;
}

export function planNameById(plans: PowerPlan[], planId: string): string {
  return plans.find((plan) => plan.id === planId)?.name ?? planId;
}

function uniquePlans(plans: PowerPlan[]): PowerPlan[] {
  const seen = new Set<string>();
  const result: PowerPlan[] = [];

  for (const plan of plans) {
    const key = normalize(plan.id);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    result.push(plan);
  }

  return result;
}

function normalize(value: string): string {
  return value.trim().toLocaleLowerCase();
}
