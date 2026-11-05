@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.js.*
import kotlin.test.*
import kotlinx.coroutines.test.runTest

@JsFun(
    "id => { const node=document.createElement('div'); node.id=id; document.body.append(node); }"
)
private external fun addSemanticsNode(id: String)

@JsFun("id => document.getElementById(id)?.remove()")
private external fun removeSemanticsNode(id: String)

@JsFun("(id,name) => document.getElementById(id).getAttribute(name)")
private external fun semanticsAttribute(id: String, name: String): String?

@JsFun(
    "id => { const observer=new MutationObserver(()=>{}); observer.observe(document.getElementById(id),{attributes:true}); return observer; }"
)
private external fun recordSemanticsMutations(id: String): JsAny

@JsFun("observer => observer.takeRecords().length")
private external fun takeSemanticsMutationCount(observer: JsAny): Int

@JsFun("observer => observer.disconnect()")
private external fun stopSemanticsMutationRecording(observer: JsAny)

@JsFun(
    """async () => {
    let progress = 0;
    await Promise.resolve().then(() => { progress |= 1; });
    await new Promise(resolve => requestAnimationFrame(() => { progress |= 2; resolve(); }));
    await new Promise(resolve => setTimeout(() => { progress |= 4; resolve(); }, 0));
    return String(progress);
}"""
)
private external fun realBrowserProgress(): Promise<JsString>

@JsFun(
    """() => {
    const host = document.createElement('div'); host.id = 'semantics-shadow-host';
    document.body.append(host);
    const root = host.attachShadow({mode:'open'});
    root.innerHTML = '<div id="shadow-control"></div>';
}"""
)
private external fun addShadowControl()

@JsFun(
    """() => {
    const root = document.getElementById('semantics-shadow-host').shadowRoot;
    root.getElementById('shadow-control').outerHTML = '<div id="shadow-control"></div>';
}"""
)
private external fun replaceShadowControl()

@JsFun(
    "() => document.getElementById('semantics-shadow-host').shadowRoot.getElementById('shadow-control').getAttribute('aria-checked')"
)
private external fun shadowControlChecked(): String?

@JsFun(
    """() => {
    let count = 0;
    const original = Document.prototype.querySelector;
    Document.prototype.querySelector = function(...args) { count++; return original.apply(this,args); };
    return {count: () => count, close: () => { Document.prototype.querySelector = original; }};
}"""
)
private external fun recordControlQueries(): JsAny

@JsFun("recorder => recorder.count()") private external fun controlQueryCount(recorder: JsAny): Int
@JsFun("recorder => recorder.close()") private external fun stopControlQueries(recorder: JsAny)

class WebSemanticsTest {
    @Test
    fun updatingOneControlDoesNotSearchEveryRegisteredControl() {
        val ids = (0 until 30).map { "semantics-cached-$it" }
        ids.forEach { id ->
            addSemanticsNode(id)
            webControl(id, "button", enabled = true, selected = false)
        }
        val recorder = recordControlQueries()
        try {
            webControl(ids.first(), "button", enabled = false, selected = true)
            assertEquals("true", semanticsAttribute(ids.first(), "aria-disabled"))
            assertEquals("true", semanticsAttribute(ids.first(), "aria-pressed"))
            assertEquals("false", semanticsAttribute(ids.last(), "aria-disabled"))
            assertEquals(0, controlQueryCount(recorder), "Connected control nodes should be cached")
        } finally {
            stopControlQueries(recorder)
            ids.forEach { forgetWebControl(it); removeSemanticsNode(it) }
        }
    }

    @Test
    fun shadowRootAddedAfterObserverStartsAndReplacedNodeReceiveSemantics() = runTest {
        val id = "semantics-existing"
        addSemanticsNode(id)
        webControl(id, "button", enabled = true, selected = null)
        try {
            addShadowControl()
            webControl("shadow-control", "checkbox", enabled = true, selected = true)
            assertEquals("true", shadowControlChecked())
            replaceShadowControl()
            realBrowserProgress().awaitBrowser()
            assertEquals("true", shadowControlChecked(), "Do not retain a disconnected cached node")
        } finally {
            forgetWebControl("shadow-control")
            forgetWebControl(id)
            removeSemanticsNode("semantics-shadow-host")
            removeSemanticsNode(id)
        }
    }

    @Test
    fun explicitAriaOverridesSettleWithoutMutationLoopAndAllowBrowserProgress() = runTest {
        val id = "web-semantics-override-test"
        addSemanticsNode(id)
        val observer = recordSemanticsMutations(id)
        try {
            fun register() =
                webControl(
                    id,
                    "button",
                    enabled = true,
                    selected = null,
                    attributes = mapOf("aria-pressed" to "true", "aria-disabled" to "true"),
                )
            register()
            assertEquals("true", semanticsAttribute(id, "aria-pressed"))
            assertEquals("true", semanticsAttribute(id, "aria-disabled"))
            takeSemanticsMutationCount(observer)
            register()
            // Assert synchronously before yielding: the old implementation would
            // otherwise starve the event loop, preventing test cleanup as well.
            assertEquals(
                0,
                takeSemanticsMutationCount(observer),
                "Reapplying final values must not mutate DOM",
            )
            assertEquals("7", realBrowserProgress().awaitBrowser().toString())
            assertEquals("true", semanticsAttribute(id, "aria-pressed"))
            assertEquals("true", semanticsAttribute(id, "aria-disabled"))
            assertEquals(
                0,
                takeSemanticsMutationCount(observer),
                "The platform observer must settle",
            )
        } finally {
            forgetWebControl(id)
            stopSemanticsMutationRecording(observer)
            removeSemanticsNode(id)
        }
    }

    @Test
    fun checkboxAndRadioExposeCheckedInsteadOfSelectedOrPressed() {
        for (role in listOf("checkbox", "radio")) {
            val id = "web-semantics-$role-test"
            addSemanticsNode(id)
            try {
                webControl(id, role, enabled = true, selected = true)
                assertEquals(role, semanticsAttribute(id, "role"))
                assertEquals("true", semanticsAttribute(id, "aria-checked"))
                assertNull(semanticsAttribute(id, "aria-selected"))
                assertNull(semanticsAttribute(id, "aria-pressed"))
                webControl(id, role, enabled = true, selected = false)
                assertEquals("false", semanticsAttribute(id, "aria-checked"))
                assertNull(semanticsAttribute(id, "aria-selected"))
            } finally {
                forgetWebControl(id)
                removeSemanticsNode(id)
            }
        }
    }
}
