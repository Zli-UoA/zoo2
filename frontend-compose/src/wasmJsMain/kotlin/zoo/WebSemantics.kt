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
    "(update) => { const observed = new WeakSet(); const scan = root => { if (!observed.has(root)) { observed.add(root); observer.observe(root, {childList:true, subtree:true, attributes:true, attributeFilter:['role','tabindex','aria-disabled','aria-selected','aria-pressed','aria-expanded','aria-checked','aria-busy']}); } for (const node of root.querySelectorAll('*')) if (node.shadowRoot) scan(node.shadowRoot); }; const observer = new MutationObserver(() => { scan(document); update(); }); scan(document); return {refresh: () => scan(document), close: () => observer.disconnect()}; }"
)
private external fun observeControls(update: () -> Unit): JsAny

@JsFun("(observer) => observer.refresh()") private external fun refreshObserver(observer: JsAny)

@JsFun("(observer) => observer.close()") private external fun closeObserver(observer: JsAny)

@JsFun(
    "(node,id,enabled,focus) => { const nodes = globalThis.__zooNativeControlFocus ??= new Map(); let entry = nodes.get(id); if (entry?.node !== node) { if(entry) entry.node.removeEventListener('focus',entry.listener); entry={node,focus}; entry.listener=()=>{ entry.focus(); if (!node.isContentEditable && !node.querySelector('[contenteditable=true],input,textarea,select')) { const canvases=[...node.getRootNode().querySelectorAll('canvas')].filter(canvas=>canvas.getClientRects().length); if(canvases.length===1) canvases[0].focus({preventScroll:true}); } }; nodes.set(id,entry); node.addEventListener('focus',entry.listener); } entry.focus=focus; const index=enabled ? 0 : -1; if(node.tabIndex!==index) node.tabIndex=index; }"
)
private external fun bindControlFocus(node: JsAny, id: String, enabled: Boolean, focus: () -> Unit)

@JsFun(
    "id => {const nodes=globalThis.__zooNativeControlFocus;const entry=nodes?.get(id);if(entry){entry.node.removeEventListener('focus',entry.listener);nodes.delete(id);}}"
)
private external fun unbindControlFocus(id: String)

private data class Control(
    val role: String,
    val enabled: Boolean,
    val selected: Boolean?,
    val expanded: Boolean?,
    val attributes: Map<String, String>,
)

private val controls = mutableMapOf<String, Control>()
private val focusCallbacks = mutableMapOf<String, () -> Unit>()
private var observer: JsAny? = null

private fun applyControls() {
    controls.forEach { (id, state) ->
        controlNode(id)?.let { node ->
            val values =
                linkedMapOf<String, String?>(
                    "role" to state.role,
                    "data-status" to null,
                    "aria-expanded" to state.expanded?.toString(),
                    "aria-disabled" to (!state.enabled).toString(),
                    "aria-selected" to
                        if (
                            state.role in
                                listOf(
                                    "button",
                                    "menuitemradio",
                                    "menuitemcheckbox",
                                    "radio",
                                    "checkbox",
                                    "switch",
                                )
                        )
                            null
                        else state.selected?.toString(),
                    "aria-checked" to
                        if (
                            state.role in
                                listOf(
                                    "menuitemradio",
                                    "menuitemcheckbox",
                                    "radio",
                                    "checkbox",
                                    "switch",
                                )
                        )
                            state.selected?.toString()
                        else null,
                    "aria-pressed" to
                        if (state.role == "button") state.selected?.toString() else null,
                )
            // Resolve overrides before touching DOM. Setting then removing the same
            // observed ARIA attribute would continuously retrigger the observer.
            state.attributes
                .filterKeys {
                    it.startsWith("aria-") ||
                        it == "href" ||
                        it == "title" ||
                        it == "datetime" ||
                        it == "data-status" ||
                        it == "target" ||
                        it == "rel"
                }
                .forEach { (key, value) -> values[key] = value }
            values.forEach { (key, value) -> attribute(node, key, value) }
            focusCallbacks[id]?.let { focus -> bindControlFocus(node, id, state.enabled, focus) }
        }
    }
}

/** Compose 1.9's Web projection omits disabled and selected and flattens tab roles. */
internal fun webControl(
    id: String,
    role: String,
    enabled: Boolean,
    selected: Boolean?,
    expanded: Boolean? = null,
    attributes: Map<String, String> = emptyMap(),
) {
    controls[id] = Control(role, enabled, selected, expanded, attributes)
    if (observer == null) observer = observeControls(::applyControls)
    observer?.let(::refreshObserver)
    applyControls()
}

internal fun webControlFocus(id: String, focus: () -> Unit) {
    focusCallbacks[id] = focus
    applyControls()
}

internal fun forgetWebControl(id: String) {
    focusCallbacks.remove(id)
    unbindControlFocus(id)
    controls.remove(id)
    if (controls.isEmpty()) {
        observer?.let(::closeObserver)
        observer = null
    }
}
