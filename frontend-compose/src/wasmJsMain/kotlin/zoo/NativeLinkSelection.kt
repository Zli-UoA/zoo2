@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun

/** Selection is observed synchronously during the originating pointer/keyboard event. */
@JsFun(
    """() => {
    const selection = globalThis.getSelection?.()?.toString() ?? '';
    if (selection) return selection;
    let active = document.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
        const start = active.selectionStart, end = active.selectionEnd;
        if (start !== null && end !== null && start !== end) return active.value.slice(start, end);
    }
    return '';
}"""
)
private external fun selectedText(): String

internal actual fun nativeSelectedText(): String = selectedText()

@JsFun("(url) => new URL(url, globalThis.location.href).href")
private external fun absoluteUrl(url: String): String

internal actual fun nativeAbsoluteUrl(url: String): String = absoluteUrl(url)
