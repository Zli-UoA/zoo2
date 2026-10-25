# Native authentication and push deployment

## Current boundary

The implementation includes HTTPS browser authentication, Apple association metadata,
APNs registration/revocation, provider token signing, and native alert delivery.
Production authentication and APNs delivery have not been verified. An ordinary
AuthZ user cannot register clients: AuthZ disables dynamic registration and requires
an administrator session. Public source access does not grant this permission.

The Apple App ID already has Push Notifications and Associated Domains enabled.
Sandbox and Production topic-specific APNs keys have been created with the account
owner's approval and saved locally outside the repository with owner-only access.
The dedicated public OAuth client is registered with Client ID
`UPPfdYapcgiBgnfZIoGvFuJMAEIhXQOV` and is configured as the iOS default.
No production database migration or service deployment has been performed.

Coordination requests:
- [AuthZ client registration](https://github.com/Zli-UoA/AuthZ/issues/82)
- [Kubernetes/Flux deployment](https://github.com/Zli-UoA/zoo2-infra/issues/60)

## AuthZ administrator handoff

The public OAuth client was registered in AuthZ with these settings:

| Field | Value |
| --- | --- |
| Display name | Zoo for iOS |
| Client type | Public; no client secret |
| Redirect URI | `https://zli.chat/ios/oauth/callback` |
| Grants | `authorization_code`, `refresh_token` |
| Response type | `code` |
| PKCE | Required, `S256` |
| Token endpoint authentication | `none` |
| Allowed scopes | `openid profile offline_access` |
| Linked resource / audience | `https://zli.chat/api` |
| Issuer | `https://account.zli.works/api/auth` |

The public Client ID is included in the app defaults and remains editable in
Connection settings. Do not reuse
Zoo's browser client or add its secret to the app. The previous custom-scheme
redirect is incompatible with AuthZ's production HTTPS-only redirect validation.
The app also checks AuthZ's RFC 9207 `iss` authorization response parameter.

## Apple account and application

The supplied Team ID is `F82XB5KFX2`; the bundle identifier is `works.zli.zoo`.
Register that explicit App ID with Push Notifications and Associated Domains.
Create signing/provisioning assets for this team. If the App ID prefix differs
from the Team ID (legacy accounts), correct the association document to the actual
signed application identifier before deployment.

The iOS 18 HTTPS `ASWebAuthenticationSession` callback requires the
`webcredentials:zli.chat` associated domain. The backend serves
`/.well-known/apple-app-site-association` as JSON with
`webcredentials.apps = ["F82XB5KFX2.works.zli.zoo"]`. Serve this over public HTTPS
without redirects or authentication. The proxy routes this path and the OAuth
fallback page to the backend. Query strings are excluded from backend request
spans, and the fallback page never reflects codes or states. If upstream access
logging is enabled, redact callback query strings there as well.

Use separate Sandbox and Production APNs token signing keys, each restricted to
the topic `works.zli.zoo`.
Store the downloaded `.p8` only in the deployment secret manager; never commit it
or paste it in a chat, PR, screenshot or client configuration. Keep the Key ID
and Team ID with the server configuration. No Apple account password is needed
by the app or backend.

## Server deployment settings

| Environment variable | Value |
| --- | --- |
| `APPLE_TEAM_ID` | `F82XB5KFX2` |
| `APNS_SANDBOX_KEY_ID` | The 10-character ID of the Sandbox APNs key |
| `APNS_SANDBOX_PRIVATE_KEY` | Sandbox multiline P-256 `.p8` PEM secret |
| `APNS_PRODUCTION_KEY_ID` | The 10-character ID of the Production APNs key |
| `APNS_PRODUCTION_PRIVATE_KEY` | Production multiline P-256 `.p8` PEM secret |

The backend uses a fixed APNs topic `works.zli.zoo` and Apple's sandbox/production
hosts. It does not accept arbitrary provider URLs. APNs credentials remain in the
backend. Missing APNs keys disable registration and delivery; Web Push continues
independently. Invalid partial configuration fails closed.

Review and apply `20261026000000_create_apns_subscriptions.sql` through the normal
Atlas deployment workflow before running the new backend. It adds a table and
index, plus an expiring revocation table; it does not modify existing notification records. Check the migration on
a staging database before approving a production deployment. The compose file
passes the APNs configuration into the backend and supplies the public Team ID.
Outbound HTTPS/HTTP2 access to Apple's APNs endpoints is required.

Production is managed by `Zli-UoA/zoo2-infra` using Kubernetes/Flux, not this
repository's compose file. Add the exact callback/association routes to its
IngressRoute and APNs environment variables through a backend-only Secret.
Preserve the migrate-before-app dependency and preview namespace isolation.
The infrastructure issue above lists the inspected paths and rollout checks.

## Device behavior

The Profile screen requests system permission only when the user enables push.
Debug builds use the APNs sandbox; Release uses production. The Info.plist value
and `aps-environment` entitlement derive from the same build setting. Verify the
signed provisioning profile agrees with this setting, including exported builds.
APNs registration must use a real signed device; Simulator delivery is not an
acceptance test for this server path.

The server produces an alert notification, which iOS can display while the app is
backgrounded or terminated. No silent background processing is claimed. A tap
waits for authenticated workspace data before opening its channel/message; backend
membership checks still determine access. Alerts contain generic text so private
message previews are not retained on a locked device.

Each registration has a random 256-bit revocation capability saved to the device
Keychain before sending the request. Only its hash is stored in the database.
The app serializes registration and revocation, preserving the capability after
network failure. Logout disables local registration and queues server cleanup;
an offline logout cannot guarantee immediate removal of the server registration.
Registration and revocation share a database transaction lock. Revocation
records prevent requests that arrive after logout from recreating a registration.
Registration requests retain their original timestamp and expire after 10 minutes
(with up to 30 seconds of future clock skew). Revocations are retained for 11
minutes, capped at 10,000 records across backend replicas, and limited to 120
requests/minute per backend process. Cleanup removes expired records as part of
subsequent revocations; no unbounded client-address map is retained. A 429 response
keeps the device's receipt for retry. Devices with incorrect clocks must enable
automatic date/time. Expired receipts rotate only after successful revocation.

Cleanup is retried on the next foreground connection or launch without requiring
an unexpired access token. A delayed old revocation cannot remove a new account's
registration. Reinstalling the app or losing its Keychain may require server-side
removal of an orphaned registration; it cannot silently transfer device ownership.

## Release verification

1. Deploy the migration, backend, proxy routes and association document to staging.
2. Configure the public OAuth Client ID and verify browser login, callback, user
   identity, API audience, refresh and logout on a signed physical device.
3. Enable push in Profile. Verify one device registration with a hashed capability;
   never display raw device tokens or capabilities in logs or screenshots.
4. From a second test account, send a DM, mention and thread reply while the first
   device is backgrounded. Confirm receipt, generic preview and correct tap target.
5. Repeat with the app terminated. Verify tap navigation after authentication and
   synchronization. Remove channel access and confirm protected content is denied.
6. Log out while offline, reconnect and confirm server cleanup. Change accounts and
   verify the previous account's alerts cannot open content in the new account.
7. Repeat production-environment delivery using TestFlight with a distribution
   profile. Verify invalid/expired device tokens are pruned on APNs HTTP 410.

Record actual device and deployment results separately from unit tests and demo
screenshots. Do not declare the app production-ready until these checks pass.
