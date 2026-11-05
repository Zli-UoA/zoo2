import { noAutofill } from "../../components/ui/autofill";

type Scope = Document | ShadowRoot | HTMLElement;
type ShadowListener = (host: Element, shadow: ShadowRoot) => void;
const hooks = new WeakMap<object, { listeners: Set<ShadowListener>; restore: () => void }>();

/** Watch only this Compose owner, including browser editing owners in nested shadow trees. */
export function applyComposeInputHints(scope: Scope): () => void {
  const document = scope.nodeType === 9 ? (scope as Document) : scope.ownerDocument!;
  const view = document.defaultView;
  if (!view) return () => {};
  const hints = Object.entries(noAutofill).map(
    ([key, value]) => [key === "autoComplete" ? "autocomplete" : key, String(value)] as const,
  );
  let disposed = false;
  const observed = new WeakSet<Node>();
  const observer = new view.MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "attributes") scan(record.target);
      else for (const node of record.addedNodes) scan(node);
    }
  });
  const observe = (node: Node) => {
    if (observed.has(node)) return;
    observed.add(node);
    observer.observe(node, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: hints.map(([key]) => key),
    });
  };
  const scan = (node: Node) => {
    if (disposed) return;
    if (node.nodeType === 1) {
      const element = node as Element;
      if (element.matches("input, textarea"))
        for (const [key, value] of hints)
          if (element.getAttribute(key) !== value) element.setAttribute(key, value);
      if (element.shadowRoot) {
        observe(element.shadowRoot);
        scan(element.shadowRoot);
      }
    }
    for (const child of node.childNodes) scan(child);
  };
  const contains = (node: Node): boolean => {
    let current: Node | null = node;
    while (current) {
      if (current === scope) return true;
      current =
        current.parentNode ?? (current.nodeType === 11 ? (current as ShadowRoot).host : null);
    }
    return false;
  };
  const attached: ShadowListener = (host, shadow) => {
    if (!contains(host)) return;
    observe(shadow);
    scan(shadow);
  };
  const prototype = view.Element.prototype;
  let hook = hooks.get(prototype);
  if (!hook) {
    const original = Reflect.get(prototype, "attachShadow") as typeof prototype.attachShadow;
    const listeners = new Set<ShadowListener>();
    const wrapper: typeof original = function (this: Element, options) {
      const shadow = original.call(this, options);
      for (const listener of listeners) listener(this, shadow);
      return shadow;
    };
    prototype.attachShadow = wrapper;
    hook = {
      listeners,
      restore: () => {
        if (prototype.attachShadow === wrapper) prototype.attachShadow = original;
        hooks.delete(prototype);
      },
    };
    hooks.set(prototype, hook);
  }
  hook.listeners.add(attached);
  observe(scope);
  scan(scope);
  return () => {
    if (disposed) return;
    disposed = true;
    observer.disconnect();
    hook.listeners.delete(attached);
    if (!hook.listeners.size) hook.restore();
  };
}
