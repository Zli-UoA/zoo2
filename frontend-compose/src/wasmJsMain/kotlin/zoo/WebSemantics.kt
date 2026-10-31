@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.js.JsAny

/** DOM primitives only. Roles and control state remain owned by common Kotlin UI. */
@JsFun(
    "(id) => { const find = root => { const item = root.querySelector('[id=\"' + id + '\"]'); if (item) return item; for (const node of root.querySelectorAll('*')) { if (node.shadowRoot) { const found = find(node.shadowRoot); if (found) return found; } } return null; }; return find(document); }"
)
private external fun controlNode(id: String): JsAny?

@JsFun(
    "(node, name, value) => { if (value === null) { if (node.hasAttribute(name)) node.removeAttribute(name); } else if (node.getAttribute(name) !== value) node.setAttribute(name, value); }"
)
private external fun attribute(node: JsAny, name: String, value: String?)

@JsFun(
    "(update) => { const observed = new WeakSet(); const scan = root => { if (!observed.has(root)) { observed.add(root); observer.observe(root, {childList:true, subtree:true, attributes:true, attributeFilter:['role','aria-disabled','aria-selected']}); } for (const node of root.querySelectorAll('*')) if (node.shadowRoot) scan(node.shadowRoot); }; const observer = new MutationObserver(() => { scan(document); update(); }); scan(document); return {refresh: () => scan(document), close: () => observer.disconnect()}; }"
)
private external fun observeControls(update: () -> Unit): JsAny

@JsFun("(observer) => observer.refresh()") private external fun refreshObserver(observer: JsAny)

@JsFun("(observer) => observer.close()") private external fun closeObserver(observer: JsAny)

private data class Control(val role: String, val enabled: Boolean, val selected: Boolean?)

private val controls = mutableMapOf<String, Control>()
private var observer: JsAny? = null

private fun applyControls() {
    controls.forEach { (id, state) ->
        controlNode(id)?.let { node ->
            attribute(node, "role", state.role)
            attribute(node, "aria-disabled", (!state.enabled).toString())
            attribute(node, "aria-selected", state.selected?.toString())
        }
    }
}

/** Compose 1.9's Web projection omits disabled and selected and flattens tab roles. */
internal fun webControl(id: String, role: String, enabled: Boolean, selected: Boolean?) {
    controls[id] = Control(role, enabled, selected)
    if (observer == null) observer = observeControls(::applyControls)
    observer?.let(::refreshObserver)
    applyControls()
}

internal fun forgetWebControl(id: String) {
    controls.remove(id)
    if (controls.isEmpty()) {
        observer?.let(::closeObserver)
        observer = null
    }
}
