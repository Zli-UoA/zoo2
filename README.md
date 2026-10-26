# Zoo for iOS

Zoo's iPhone client for iOS 26 and later. SwiftUI's `WebView` and `WebPage`
render the shared Zoo web interface at `https://zli.chat`. Native code owns
OAuth authentication, Keychain credentials, APNs registration, and notification navigation.

## Development

Open `Zoo.xcodeproj` in Xcode. The project is generated from `project.yml` with
XcodeGen. Use Xcode 26 or later with an SDK that includes WebKit for SwiftUI;
the migration is developed with Xcode 27.1 and Swift 6.4 in Swift 6 language mode.

```sh
make generate
make build
make test
```

Override `DESTINATION` for an available simulator or physical device. A signed
device build requires the `works.zli.zoo` App ID in team `F82XB5KFX2`, Associated
Domains, and Push Notifications capabilities. Debug uses APNs sandbox; Release
uses APNs production. Do not commit signing keys, profiles, tokens or `.env` files.

## Repositories

- `Zli-UoA/zoo2-ios`: native shell, authentication, notifications, app tests.
- `Zli-UoA/zoo2`: shared React UI, REST API, Electric synchronization, APNs backend.
- `Zli-UoA/zoo2-infra`: production routing, secrets, migrations and deployment.

The native services were extracted from `Zli-UoA/zoo2` commit
`96b1538e134cf211638ffde9a8d6a24cb5be7e16`. Duplicate SwiftUI chat screens,
repositories and Shape subscriptions are replaced by the shared Web implementation.
The Web source is not copied into this repository.

## Integration and deployment

The shared UI must include native bridge version 1 before this app can display
an authenticated workspace. See [the architecture and protocol](docs/architecture.md).
The app loads the deployed site, so a local change to the Web source alone does
not update the app. Web updates must preserve compatibility with released bridge
versions and be tested against supported iOS releases.

Production OAuth and APNs acceptance are not yet verified. The previously observed
AASA URL returned the SPA HTML rather than the association JSON, blocking HTTPS
login. See [deployment prerequisites](docs/native-auth-push.md) and
[zoo2-infra #60](https://github.com/Zli-UoA/zoo2-infra/issues/60).
This repository migration does not deploy any server changes or apply migrations.

## Validation

Unit tests cover PKCE, token renewal, registration cleanup, bridge origin checks
and notification destinations. WebPage tests run the actual JavaScript reply
bridge using local HTML fixtures; they do not log in to production or send pushes.
UI tests exercise the native welcome screen. Real-device login, message exchange,
background/terminated APNs delivery, and notification navigation remain required
before a production release.

## Migration verification (2026-10-26)

- iOS 26.5 and iOS 27.0 simulators: 22 unit/integration tests and one UI test passed on each OS.
- Signed physical-device build from this standalone repository: passed.
- Shared Web change: 467 tests, TypeScript checking, and production build passed.
- Web static checks: zero errors; eight pre-existing warnings in `scripts/pwa.test.mjs`.
- Independent `reviewer-jp` review: findings addressed and final review passed.
- Production Web deployment, AASA/callback configuration and APNs delivery remain unverified.
- Production deployment and database migrations are separate from this source migration.
