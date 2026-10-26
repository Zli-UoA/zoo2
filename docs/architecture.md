# Architecture

## Ownership

The iOS app uses SwiftUI `WebView` with a `WebPage` on iOS 26+. The React application
owns chat UI, routing, localization, API writes and Electric subscriptions. Swift
owns `ASWebAuthenticationSession`, PKCE, refresh serialization, Keychain and APNs.
The shared Web app uses its existing token-provider interface to request a native
access token. Refresh tokens and APNs revocation capabilities never cross the bridge.

`WebSession` is main-actor isolated and observed by SwiftUI. Login resolves the
current user before creating a WebPage. Logout invalidates pending bridge responses,
cancels authentication work, discards the WebPage and its nonpersistent data store,
and queues APNs cleanup. Failed/offline cleanup retains its Keychain receipt for retry.

## Bridge version 1

Only the HTTPS `zli.chat` main frame may call `window.webkit.messageHandlers.zoo`.
Every request is `{version: 1, method: string}` and returns a Promise through
`WKScriptMessageHandlerWithReply`. The native side injects the immutable
`window.__ZOO_NATIVE__ = {version: 1}` marker before document scripts run.
It validates the frame URL, security origin and protocol version, independently
of any JavaScript marker. Unknown commands fail closed.

| Method | Response | Purpose |
| --- | --- | --- |
| `session` | `{version: 1, userID}` | Select the native authenticated session |
| `token` | `{accessToken}` | Supply a short-lived bearer token to existing Web API clients |
| `ready` | `{ok: true}` | Announce that the Web router and navigation handler are ready |
| `logout` | Page is destroyed | End the native and Web session |
| `notificationState` | `{enabled, status, failed}` | Read notification preference and registration status |
| `enableNotifications` | `{enabled, status, failed}` | Request permission and queue APNs registration |
| `disableNotifications` | `{enabled, status, failed}` | Disable delivery and queue server cleanup |

`enabled` is the user's preference, not proof that APNs registration or delivery
succeeded. The asynchronous registration result updates `status` and `failed`, and
emits `zoo:native:notifications`. A foreground transition emits `zoo:native:resume`
and a focus event so the Web UI refreshes state. Web callers time out after 30 seconds.

Swift calls `window.__zooNavigate({userID, channelID, messageID?})` for notification
taps. Both sides check the destination's account and identifier shape. The Web
router opens `/channels/<channelID>?message=<messageID>` and reveals the chat pane,
even when navigating to the currently selected channel. Swift clears the queued
destination only after successful acknowledgement. API authorization remains the
source of truth for channel membership and message access.

## Security and lifecycle

Only trusted-origin navigation stays inside the app. User-initiated external links
open through the operating system; arbitrary script redirects are cancelled.
Bridge methods do not accept arbitrary URLs, scripts, credentials or Keychain keys.
A trusted-page script can obtain an access token, as the browser UI already can;
this is not isolation from a same-origin XSS. CSP and Web input handling remain
important. Tokens are never placed in navigation URLs or Web persistent storage.

The app uses native browser authentication, rather than authenticating inside the
WebView. HTTPS callback association and APNs deployment are still required.
The native shell has retry and logout controls outside the page, including when
the deployed Web UI is unavailable or incompatible. It does not claim offline chat.

## Acceptance

Verify signed iOS 26+ login, restart/token renewal, message send/sync, Japanese
keyboard layout, foreground recovery, notification tap during cold/warm launch,
account changes, and offline logout cleanup. A simulator/local-fixture result does
not establish production OAuth or APNs delivery. Distribution review remains a
separate release step.
