const androidOnly = new Set([
  'frontend/src/android-phone/engine.ts',
  'frontend/src/media-preview-entry.tsx',
  'frontend/media-preview.html',
  'frontend/vite.media-preview.config.ts',
  'frontend/vite.android-phone.config.ts',
  'frontend/scripts/android-phone-smoke.mjs',
  'frontend/scripts/media-preview-bundle.test.mjs',
]);

export const isAndroidFrontendOnly = path => androidOnly.has(path);

export function isStorybookVitestInput(path, project) {
  if (path === 'frontend/.storybook/fixtures.ts' || path.startsWith('frontend/.storybook/fixtures/')) return true;
  // Browser tests also mount the Storybook shell, decorators and profile fixtures.
  return project === 'browser' && path.startsWith('frontend/.storybook/') &&
    !/^frontend\/\.storybook\/(main\.ts|preview\.tsx|vitest\.setup\.ts|vite\.config\.ts|coverage\.json)$/.test(path);
}

export const isDedicatedPlaywrightTypecheckInput = path =>
  /^frontend\/(?:e2e\/(?:oidc|e2ee)\.spec\.ts|playwright\.(?:oidc|e2ee)\.config\.ts)$/.test(path);
