package zoo

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Android supplies this once so nested menus, dialogs and editors use the same density. */
val LocalTouchUi = staticCompositionLocalOf { false }

internal fun touchTargetSize(size: Dp, touch: Boolean): Dp =
    if (touch) maxOf(size, AndroidDesign.touchTarget) else size

/** Keep channel and DM rows aligned, including their loading placeholders. */
internal fun navigationRowHeight(touch: Boolean, narrow: Boolean): Dp =
    touchTargetSize(if (narrow) 36.dp else 32.dp, touch)

internal enum class MobileDestination(val path: String, val icon: String, val labelKey: String) {
    Home("/home", "home", "home.navTitle"),
    Channels("/", "hash", "channels.title"),
    Notifications("/notifications", "bell", "notifications.title"),
    Bubble("/bubble", "message-circle-more", "bubbleChat.navTitle"),
    Settings("/settings", "settings", "settings.navTitle"),
}

internal fun mobileDestination(path: String): MobileDestination? {
    val route = path.substringBefore('?')
    return when {
        route == "/" || route.startsWith("/channels/") -> MobileDestination.Channels
        isHomePath(route) -> MobileDestination.Home
        route == "/notifications" -> MobileDestination.Notifications
        route == "/bubble" -> MobileDestination.Bubble
        route == "/settings" || route.startsWith("/settings/") -> MobileDestination.Settings
        else -> null
    }
}

/** The back stack owns child screens; the five destinations are shown on their roots. */
internal fun showMobileNavigation(path: String, channelList: Boolean): Boolean =
    when (val route = path.substringBefore('?')) {
        "/home", "/notifications", "/bubble" -> true
        "/settings" -> io.ktor.http.Url("https://zoo.invalid$path").parameters["tab"] == null
        else -> channelList && (route == "/" || route.startsWith("/channels/"))
    }
