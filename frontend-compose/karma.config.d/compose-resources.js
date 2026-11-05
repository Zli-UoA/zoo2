// Resource APIs resolve ./composeResources from Karma's context document. Serve
// the same generated resources copied beside the test executable, without
// loading binary font files as scripts or embedding a duplicate test fixture.
// Use Compose's test resource reader before the Wasm module initializes. Tests
// still load the generated font bytes, without depending on CacheStorage.
config.files.unshift({
    pattern: require('path').resolve(
        config.basePath,
        '../../../../src/wasmJsTest/resources/compose-resources-test-init.js',
    ),
    included: true,
    served: true,
    watched: false,
});
config.files.push({
    pattern: config.basePath + '/kotlin/composeResources/**/*',
    included: false,
    served: true,
    watched: false,
});
config.proxies['/composeResources/'] = '/base/kotlin/composeResources/';

// Resource-backed Wasm tests may decode several font faces on slower CI hosts.
config.client = config.client || {};
config.client.mocha = { ...(config.client.mocha || {}), timeout: 30000 };
