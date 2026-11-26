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
