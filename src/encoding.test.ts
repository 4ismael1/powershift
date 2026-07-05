import { describe, expect, it } from 'vitest';

const mojibakePattern = new RegExp('[\\u00c3\\u00c2\\ufffd]');
const sourceModules = import.meta.glob('./**/*.{vue,ts,css}', {
  eager: true,
  query: '?raw',
  import: 'default',
}) as Record<string, string>;

describe('source encoding', () => {
  it('does not contain common UTF-8 mojibake markers', () => {
    const offenders = Object.entries(sourceModules)
      .filter(([, source]) => mojibakePattern.test(source))
      .map(([path]) => path);

    expect(offenders).toEqual([]);
  });
});
