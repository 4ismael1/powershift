import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = readFileSync(
  fileURLToPath(new URL('../scripts/collect-diagnostics.ps1', import.meta.url)),
  'utf8',
);

describe('support diagnostics privacy contract', () => {
  it('collects bounded operational diagnostics and names every excluded secret', () => {
    expect(script).toContain("'agent-state.json', 'events.jsonl', 'events.jsonl.1'");
    expect(script).toContain('Get-AuthenticodeSignature');
    expect(script).toContain('Get-ScheduledTask');
    expect(script).toContain('config.json, agent-control.token, and');
    expect(script).toContain('power-control-lease.json');
    expect(script).not.toMatch(/Copy-Item[^\r\n]*(?:config\.json|agent-control\.token|power-control-lease\.json)/i);
  });
});
