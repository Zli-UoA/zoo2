@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun

@JsFun(
    """(id, raw, close) => { const state = globalThis.__zooOutsideClicks ??= {items:new Map(), listener:null}; if(!state.listener) {state.listener = event => {const item=[...state.items.values()].at(-1); if(item && (event.clientX < item.bounds.left || event.clientX > item.bounds.right || event.clientY < item.bounds.top || event.clientY > item.bounds.bottom)) item.close();}; document.addEventListener('pointerdown',state.listener,true);} state.items.set(id,{bounds:JSON.parse(raw),close}); }"""
)
internal external fun observeOutsideClick(id: String, bounds: String, close: () -> Unit)

@JsFun(
    """id => {const state=globalThis.__zooOutsideClicks;if(!state)return;state.items.delete(id);if(!state.items.size){document.removeEventListener('pointerdown',state.listener,true);state.listener=null;} }"""
)
internal external fun removeOutsideClick(id: String)
