@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.js.JsAny

/** DOM primitives only. Roles and control state remain owned by common Kotlin UI. */
@JsFun("(observer, id) => observer.find(id)")
private external fun controlNode(observer: JsAny, id: String): JsAny?

@JsFun(
    "(node, name, value) => { if (value === null) { if (node.hasAttribute(name)) node.removeAttribute(name); } else if (node.getAttribute(name) !== value) node.setAttribute(name, value); }"
)
private external fun attribute(node: JsAny, name: String, value: String?)

@JsFun(
    """(update) => {
    const roots = new Set();
    const nodes = new Map();
    let refreshed = false;
    const scan = root => {
        if ((root.nodeType === 9 || root.nodeType === 11) && !roots.has(root)) {
            roots.add(root);
            observer.observe(root, {childList:true, subtree:true, attributes:true,
                attributeFilter:['id','role','tabindex','aria-disabled','aria-selected','aria-pressed','aria-expanded','aria-checked','aria-busy']});
        }
        for (const node of root.querySelectorAll('*')) if (node.shadowRoot) scan(node.shadowRoot);
    };
    const refresh = () => {
        // A shadow root attached to an existing host emits no document mutation.
        // Discover these once per synchronous batch, rather than once per control.
        if (refreshed) return;
        refreshed = true;
        queueMicrotask(() => { refreshed = false; });
        scan(document);
    };
    const observer = new MutationObserver(records => {
        let structural = false;
        const changed = new Set();
        for (const record of records) {
            if (record.type === 'childList' || record.attributeName === 'id') {
                structural = true;
                for (const node of record.addedNodes ?? []) {
                    if (node.nodeType !== 1) continue;
                    if (node.shadowRoot) scan(node.shadowRoot);
                    scan(node);
                }
            } else if (record.target.id) changed.add(record.target.id);
        }
        if (structural) {
            for (const [id, node] of nodes) if (!node.isConnected || node.id !== id) nodes.delete(id);
            for (const root of roots) if (root.host && !root.host.isConnected) roots.delete(root);
            update('');
        } else for (const id of changed) update(id);
    });
    scan(document);
    return {
        refresh,
        find: id => {
            const cached = nodes.get(id);
            if (cached?.isConnected && cached.id === id) return cached;
            nodes.delete(id);
            const find = () => {
                for (const root of roots) {
                    if (root.host && !root.host.isConnected) continue;
                    const node = root.querySelector('[id="' + CSS.escape(id) + '"]');
                    if (node) { nodes.set(id, node); return node; }
                }
                return null;
            };
            const node = find();
            if (node) return node;
            // A second host can acquire a shadow root within the same batch.
            scan(document);
            return find();
        },
        forget: id => nodes.delete(id),
        close: () => { observer.disconnect(); roots.clear(); nodes.clear(); }
    };
}"""
)
private external fun observeControls(update: (String) -> Unit): JsAny

@JsFun("(observer) => observer.refresh()") private external fun refreshObserver(observer: JsAny)

@JsFun("(observer, id) => observer.forget(id)")
private external fun forgetControlNode(observer: JsAny, id: String)

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

private fun applyControls(id: String = "") {
    if (id.isEmpty()) controls.keys.toList().forEach(::applyControl)
    else applyControl(id)
}

private fun applyControl(id: String) {
    val state = controls[id] ?: return
    val node = observer?.let { controlNode(it, id) } ?: return
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
    applyControl(id)
}

internal fun webControlFocus(id: String, focus: () -> Unit) {
    focusCallbacks[id] = focus
    applyControl(id)
}

internal fun forgetWebControl(id: String) {
    focusCallbacks.remove(id)
    unbindControlFocus(id)
    controls.remove(id)
    observer?.let { forgetControlNode(it, id) }
    if (controls.isEmpty()) {
        observer?.let(::closeObserver)
        observer = null
    }
}
