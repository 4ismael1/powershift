import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const releaseWorkflow = readFileSync(
  fileURLToPath(new URL('../.github/workflows/release.yml', import.meta.url)),
  'utf8',
);
const ciWorkflow = readFileSync(
  fileURLToPath(new URL('../.github/workflows/ci.yml', import.meta.url)),
  'utf8',
);
const nodeVersion = readFileSync(fileURLToPath(new URL('../.node-version', import.meta.url)), 'utf8').trim();
const rustToolchain = readFileSync(
  fileURLToPath(new URL('../rust-toolchain.toml', import.meta.url)),
  'utf8',
);
const versionVerifier = readFileSync(
  fileURLToPath(new URL('../scripts/verify-version.ps1', import.meta.url)),
  'utf8',
);

describe('release supply-chain contract', () => {
  it('pins every GitHub Action to an immutable commit', () => {
    const uses = [...`${releaseWorkflow}\n${ciWorkflow}`.matchAll(/^\s*uses:\s*([^\s#]+)/gm)].map(
      (match) => match[1],
    );

    expect(uses.length).toBeGreaterThan(0);
    for (const action of uses) {
      expect(action).toMatch(/@[a-f0-9]{40}$/);
    }
  });

  it('signs helpers before bundling and verifies every shipped executable', () => {
    const helperBuild = releaseWorkflow.indexOf('Build and sign native background executables');
    const bundleBuild = releaseWorkflow.indexOf('Build signed application and installer');

    expect(helperBuild).toBeGreaterThan(0);
    expect(bundleBuild).toBeGreaterThan(helperBuild);
    expect(releaseWorkflow).toContain("beforeBuildCommand = 'npm run build:frontend'");
    for (const binary of [
      'powershift.exe',
      'powershift-agent.exe',
      'powershift-tray.exe',
      'bundle\\nsis\\*.exe',
    ]) {
      expect(releaseWorkflow).toContain(binary);
    }
    expect(releaseWorkflow).toContain("$signature.Status -ne 'Valid'");
  });

  it('fails closed without signing secrets and publishes integrity metadata', () => {
    for (const secret of [
      'WINDOWS_CERTIFICATE_BASE64',
      'WINDOWS_CERTIFICATE_PASSWORD',
      'WINDOWS_TIMESTAMP_URL',
    ]) {
      expect(releaseWorkflow).toContain(secret);
    }
    expect(releaseWorkflow).toContain('Get-FileHash');
    expect(releaseWorkflow).toContain('actions/attest@');
    expect(releaseWorkflow).toContain('verify-version.ps1');
    expect(ciWorkflow).toContain('cargo audit -D warnings');
    expect(releaseWorkflow).toContain('cargo audit -D warnings');
  });

  it('pins release toolchains instead of following moving channels', () => {
    expect(nodeVersion).toBe('22.23.1');
    expect(rustToolchain).toContain('channel = "1.97.1"');
    expect(ciWorkflow).toContain("node-version: '22.23.1'");
    expect(releaseWorkflow).toContain("node-version: '22.23.1'");
    expect(ciWorkflow).toContain('rustup toolchain install 1.97.1');
    expect(releaseWorkflow).toContain('rustup toolchain install 1.97.1');
    expect(`${ciWorkflow}\n${releaseWorkflow}`).not.toContain('rustup update stable');
  });

  it('validates a GitHub ref as a version only when the ref is a tag', () => {
    expect(versionVerifier).toContain("$env:GITHUB_REF_TYPE -eq 'tag'");
    expect(versionVerifier).toContain('$env:GITHUB_REF_NAME');
    expect(releaseWorkflow).toContain("-Tag '${{ github.ref_name }}'");
  });
});
