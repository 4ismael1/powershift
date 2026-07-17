import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const ciWorkflow = readFileSync(
  fileURLToPath(new URL('../.github/workflows/ci.yml', import.meta.url)),
  'utf8',
);
const signedReleaseWorkflow = fileURLToPath(
  new URL('../.github/workflows/release.yml', import.meta.url),
);
const nodeVersion = readFileSync(
  fileURLToPath(new URL('../.node-version', import.meta.url)),
  'utf8',
).trim();
const rustToolchain = readFileSync(
  fileURLToPath(new URL('../rust-toolchain.toml', import.meta.url)),
  'utf8',
);
const versionVerifier = readFileSync(
  fileURLToPath(new URL('../scripts/verify-version.ps1', import.meta.url)),
  'utf8',
);

describe('CI supply-chain contract', () => {
  it('does not configure a signing workflow', () => {
    expect(existsSync(signedReleaseWorkflow)).toBe(false);
  });

  it('pins every GitHub Action to an immutable commit', () => {
    const uses = [...ciWorkflow.matchAll(/^\s*uses:\s*([^\s#]+)/gm)].map((match) => match[1]);

    expect(uses.length).toBeGreaterThan(0);
    for (const action of uses) {
      expect(action).toMatch(/@[a-f0-9]{40}$/);
    }
  });

  it('builds native bundle resources before linting the Tauri host', () => {
    const prepare = ciWorkflow.indexOf('npm run prepare:native');
    const clippy = ciWorkflow.indexOf('cargo clippy --workspace --all-targets');

    expect(prepare).toBeGreaterThan(0);
    expect(clippy).toBeGreaterThan(prepare);
  });

  it('audits dependencies and publishes an explicitly unsigned CI artifact', () => {
    expect(ciWorkflow).toContain('npm audit --audit-level=high');
    expect(ciWorkflow).toContain('cargo audit -D warnings');
    expect(ciWorkflow).toContain('npm run tauri -- build');
    expect(ciWorkflow).toContain('Upload unsigned installer');
    expect(ciWorkflow).toContain('powershift-unsigned-windows-installer');
  });

  it('pins CI toolchains instead of following moving channels', () => {
    expect(nodeVersion).toBe('22.23.1');
    expect(rustToolchain).toContain('channel = "1.97.1"');
    expect(ciWorkflow).toContain("node-version: '22.23.1'");
    expect(ciWorkflow).toContain('rustup toolchain install 1.97.1');
    expect(ciWorkflow).not.toContain('rustup update stable');
  });

  it('validates a GitHub ref as a version only when the ref is a tag', () => {
    expect(versionVerifier).toContain("$env:GITHUB_REF_TYPE -eq 'tag'");
    expect(versionVerifier).toContain('$env:GITHUB_REF_NAME');
  });
});
