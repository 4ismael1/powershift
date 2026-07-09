import { describe, expect, it } from 'vitest';
import appSource from './App.vue?raw';

describe('agent state refresh model', () => {
  it('uses native change notifications with only a slow reconciliation fallback', () => {
    expect(appSource).toContain("listen('powershift://agent-state-changed'");
    expect(appSource).toContain('30_000');
    expect(appSource).not.toContain('}, 2500)');
  });

  it('coalesces a change that arrives during an in-flight refresh', () => {
    expect(appSource).toContain('agentSnapshotPending = true');
    expect(appSource).toContain('if (agentSnapshotPending)');
  });
});
