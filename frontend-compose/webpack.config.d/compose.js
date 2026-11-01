// Browser hosts may expose a partial process polyfill. Kotlin 2.2's Wasm loader
// otherwise assumes process.release exists and mistakes that polyfill for Node.
config.plugins.push(new (require('webpack')).DefinePlugin({
    'process.release.name': JSON.stringify('browser')
}));
// Karma supplies its own output object and resource path.
if (config.output) config.output.publicPath = '/compose/';
config.devtool = 'source-map';
