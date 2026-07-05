import { describe, expect, it, vi } from 'vitest';
import {
  getActivePowerPlan,
  getPowerPlans,
  planNameById,
  resolvePlanId,
  setActivePowerPlan,
  toPowerPlanOptions,
  type InvokeFn,
  type PowerPlan,
} from './powerApi';

const plans: PowerPlan[] = [
  { id: '381b4222-f694-41f0-9685-ff5bb260df2e', name: 'Equilibrado' },
  { id: '8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c', name: 'Alto rendimiento' },
];

describe('powerApi', () => {
  it('calls the Tauri command for listing power plans', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(plans);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await getPowerPlans(invokeFn);

    expect(mockInvoke).toHaveBeenCalledWith('get_power_plans');
    expect(result).toEqual(plans);
  });

  it('calls the Tauri command for the active power plan', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(plans[0]);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await getActivePowerPlan(invokeFn);

    expect(mockInvoke).toHaveBeenCalledWith('get_active_power_plan');
    expect(result).toEqual(plans[0]);
  });

  it('calls the Tauri command for setting the active plan with snake_case args', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(undefined);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    await setActivePowerPlan(invokeFn, 'high');

    expect(mockInvoke).toHaveBeenCalledWith('set_active_power_plan', { plan_id: 'high' });
  });

  it('resolves a selected value by exact plan id', () => {
    expect(resolvePlanId(plans, '8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c')).toBe(
      '8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c',
    );
  });

  it('resolves a selected value by plan name case-insensitively', () => {
    expect(resolvePlanId(plans, 'alto rendimiento')).toBe('8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c');
  });

  it('returns undefined for unknown or blank selected values', () => {
    expect(resolvePlanId(plans, 'No existe')).toBeUndefined();
    expect(resolvePlanId(plans, '   ')).toBeUndefined();
  });

  it('uses real power plans as options when available and removes duplicate ids', () => {
    const result = toPowerPlanOptions([plans[0], plans[0], plans[1]]);

    expect(result).toEqual(plans);
  });

  it('uses fallback options when no real plans are available', () => {
    const result = toPowerPlanOptions([], ['Equilibrado', 'Alto rendimiento']);

    expect(result).toEqual([
      { id: 'Equilibrado', name: 'Equilibrado' },
      { id: 'Alto rendimiento', name: 'Alto rendimiento' },
    ]);
  });

  it('returns a plan name by id and falls back to the id itself', () => {
    expect(planNameById(plans, plans[0].id)).toBe('Equilibrado');
    expect(planNameById(plans, 'custom')).toBe('custom');
  });
});
