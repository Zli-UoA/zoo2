package zoo.android

import zoo.Session
import zoo.android.ui.AndroidInteractionSettings
import zoo.android.ui.AndroidUiAction

internal class InteractionSettingsBinding(private val session: Session) {
    fun snapshot(): AndroidInteractionSettings = with(session.env) {
        AndroidInteractionSettings(swipeReply, operationHaptics, reducedMotion)
    }

    fun dispatch(action: AndroidUiAction.Toggle): Boolean {
        when (action.id) {
            "swipe" -> if (session.env.swipeReply != action.value) session.platform.action("toggleSwipeReply")
            "haptics" -> if (session.env.operationHaptics != action.value) session.platform.action("toggleOperationHaptics")
            else -> return false
        }
        return true
    }
}
