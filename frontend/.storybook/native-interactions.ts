import { waitFor, queryAllByRole, queryAllByText } from "@testing-library/react";
import { expect } from "storybook/test";

type Scope = {
  role?: string;
  name?: string;
  text?: string;
  exact?: boolean;
  includeHidden?: boolean;
};
export type NativeInteraction = Scope & {
  action:
    | "click"
    | "select"
    | "type"
    | "key"
    | "wait"
    | "hover"
    | "focus"
    | "paste"
    | "callback"
    | "scroll"
    | "fixture"
    | "pointer"
    | "navigation"
    | "overflow";
  deltaY?: number;
  ctrlKey?: boolean;
  newTab?: boolean;
  urlIncludes?: string;
  sameWidthAs?: Scope;
  sameHeightAs?: Scope;
  containedBy?: Scope;
  includeHidden?: boolean;
  value?: string;
  valueExact?: string;
  count?: number;
  checked?: boolean | "mixed";
  attributes?: Record<string, string | null>;
  attributeIncludes?: Record<string, string>;
  minAttributes?: Record<string, number>;
  all?: boolean;
  width?: number;
  height?: number;
  minWidth?: number;
  maxWidth?: number;
  minHeight?: number;
  maxHeight?: number;
  args?: unknown[];
  key?: string;
  within?: Scope;
  directWithin?: boolean;
  index?: number;
  state?: "hidden" | "visible" | "present";
  enabled?: boolean;
  pressed?: boolean;
  busy?: boolean;
  file?: { name: string; type: string; base64: string };
};

const escaped = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
function roots(document: Document): (HTMLElement | ShadowRoot)[] {
  const result: (HTMLElement | ShadowRoot)[] = [document.body];
  for (let index = 0; index < result.length; index++)
    for (const element of result[index].querySelectorAll("*"))
      if (element.shadowRoot) result.push(element.shadowRoot);
  return result;
}
function windowOf(frame: HTMLIFrameElement): Window & typeof globalThis {
  const document = frame.contentDocument;
  if (!frame.isConnected || !document?.defaultView)
    throw new Error("Kotlin preview was disposed during its interaction");
  return document.defaultView as Window & typeof globalThis;
}
// Compose's transparent editor is the browser input owner of the projected control,
// not a second logical textbox. Keep querying the semantics projection while typing
// through activeInput(), including on browsers where this owner remains in the AX tree.
function isComposeBackingInput(element: HTMLElement): boolean {
  return (
    (element.tagName === "INPUT" || element.tagName === "TEXTAREA") &&
    (element.style.top.includes("--compose-internal-web-backing-input-") ||
      element.style.width.includes("--compose-internal-web-backing-input-"))
  );
}
function match(document: Document, scope: Scope): HTMLElement[] {
  const result = new Set<HTMLElement>();
  for (const root of roots(document)) {
    const name =
      scope.name === undefined
        ? undefined
        : scope.exact === false
          ? new RegExp(escaped(scope.name))
          : scope.name;
    const candidates = scope.role
      ? queryAllByRole(
          root as HTMLElement,
          scope.role,
          name === undefined
            ? { hidden: scope.includeHidden }
            : { name, hidden: scope.includeHidden },
        )
      : scope.text
        ? queryAllByText(root as HTMLElement, scope.text, { exact: scope.exact !== false })
        : [];
    candidates.forEach((element) => {
      if (element.isConnected && !isComposeBackingInput(element)) result.add(element);
    });
  }
  return [...result];
}
function composedParent(element: Element): Element | null {
  return (
    element.parentElement ??
    ("host" in element.getRootNode() ? (element.getRootNode() as ShadowRoot).host : null)
  );
}
function contains(parent: Element, child: Element): boolean {
  let current: Element | null = child;
  while (current) {
    if (current === parent) return true;
    current = composedParent(current);
  }
  return false;
}
function narrow(
  document: Document,
  candidates: HTMLElement[],
  scope?: Scope,
  direct = false,
): HTMLElement[] {
  if (!scope) return candidates;
  const scopes = match(document, scope);
  if (scope.role)
    return candidates.filter((candidate) => {
      if (!direct) return scopes.some((parent) => contains(parent, candidate));
      let parent = composedParent(candidate);
      while (parent) {
        if (parent.getAttribute("role") === scope.role)
          return scopes.includes(parent as HTMLElement);
        parent = composedParent(parent);
      }
      return false;
    });
  const selected = new Set<HTMLElement>();
  for (const label of scopes) {
    let parent: Element | null = label;
    while (parent) {
      const children = candidates.filter((candidate) => contains(parent!, candidate));
      if (children.length === 1) {
        selected.add(children[0]);
        break;
      }
      if (children.length > 1) {
        const bounds = label.getBoundingClientRect();
        children.forEach((candidate) => {
          const row = candidate.getBoundingClientRect();
          if (row.bottom > bounds.top && row.top < bounds.bottom) selected.add(candidate);
        });
        break;
      }
      parent = composedParent(parent);
    }
  }
  return [...selected];
}
function enabled(element: HTMLElement): boolean {
  return !element.hasAttribute("disabled") && element.getAttribute("aria-disabled") !== "true";
}
function activeInput(document: Document): HTMLInputElement | HTMLTextAreaElement | null {
  let active = document.activeElement;
  while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
  const win = document.defaultView as Window & typeof globalThis;
  return active instanceof win.HTMLInputElement || active instanceof win.HTMLTextAreaElement
    ? active
    : null;
}
function target(document: Document, step: NativeInteraction): HTMLElement {
  const matches = narrow(
    document,
    match(document, {
      role: step.role,
      name: step.name,
      text: step.role ? undefined : step.text,
      exact: step.exact,
      includeHidden: step.includeHidden,
    }),
    step.within,
    step.directWithin,
  );
  if (step.index !== undefined) {
    if (!Number.isInteger(step.index) || step.index < 0)
      throw new Error("Native interaction index must be a nonnegative integer");
    if (matches[step.index]) return matches[step.index];
  } else if (matches.length === 1) return matches[0];
  throw new Error(
    `Native ${step.action}: expected a unique ${step.role ?? "text"} ${step.name ?? step.text ?? ""}; found ${matches.length}${step.index === undefined ? "" : ` (index ${step.index})`}`,
  );
}
function valueOf(element: HTMLElement): string {
  if ("value" in element && typeof element.value === "string") return element.value;
  // Compose assigns EditableText through innerText, creating BR nodes. Reading
  // innerText back can collapse trailing spaces under normal CSS white-space.
  const projectedText = (node: Node): string =>
    node.nodeName === "BR"
      ? "\n"
      : node.nodeType === 3
        ? (node.textContent ?? "")
        : Array.from(node.childNodes).map(projectedText).join("");
  return element.getAttribute("aria-valuetext") ?? projectedText(element);
}

function assertWait(
  current: HTMLElement,
  step: NativeInteraction,
  win: Window & typeof globalThis,
) {
  if (
    step.state !== "present" &&
    (!current.getClientRects().length || win.getComputedStyle(current).visibility === "hidden")
  )
    throw new Error("Native control is not visible");
  if (step.enabled !== undefined && enabled(current) !== step.enabled)
    throw new Error("Native enabled state differs");
  if (step.pressed !== undefined && current.getAttribute("aria-pressed") !== String(step.pressed))
    throw new Error("Native pressed state differs");
  if (step.busy !== undefined && (current.getAttribute("aria-busy") === "true") !== step.busy)
    throw new Error("Native busy state differs");
  if (step.checked !== undefined) {
    const checked =
      "checked" in current ? String(current.checked) : current.getAttribute("aria-checked");
    if (checked !== String(step.checked)) throw new Error("Native checked state differs");
  }
  if (
    step.value !== undefined &&
    (step.value === "" ? valueOf(current) !== "" : !valueOf(current).includes(step.value))
  )
    throw new Error("Native value differs");
  if (step.valueExact !== undefined && valueOf(current) !== step.valueExact)
    throw new Error("Native exact value differs");
  for (const [name, value] of Object.entries(step.attributes ?? {}))
    if (current.getAttribute(name) !== value) throw new Error(`Native attribute ${name} differs`);
  for (const [name, value] of Object.entries(step.attributeIncludes ?? {}))
    if (!current.getAttribute(name)?.includes(value))
      throw new Error(`Native attribute ${name} does not include ${value}`);
  for (const [name, value] of Object.entries(step.minAttributes ?? {})) {
    const actual = current.getAttribute(name);
    if (actual === null || !Number.isFinite(Number(actual)) || Number(actual) < value)
      throw new Error(`Native attribute ${name} is below ${value}`);
  }
  const bounds = current.getBoundingClientRect();
  for (const [axis, scope] of [
    ["width", step.sameWidthAs],
    ["height", step.sameHeightAs],
  ] as const) {
    if (!scope) continue;
    const reference = target(current.ownerDocument, { ...scope, action: "wait" });
    if (Math.abs(bounds[axis] - reference.getBoundingClientRect()[axis]) > 0.5)
      throw new Error(`Native ${axis} differs from reference`);
  }
  if (step.containedBy) {
    const reference = target(current.ownerDocument, {
      ...step.containedBy,
      action: "wait",
    }).getBoundingClientRect();
    if (
      bounds.left < reference.left - 0.5 ||
      bounds.top < reference.top - 0.5 ||
      bounds.right > reference.right + 0.5 ||
      bounds.bottom > reference.bottom + 0.5
    )
      throw new Error("Native control extends outside its containing region");
  }
  for (const axis of ["width", "height"] as const) {
    const actual = bounds[axis];
    const exact = step[axis];
    const minimum = axis === "width" ? step.minWidth : step.minHeight;
    const maximum = axis === "width" ? step.maxWidth : step.maxHeight;
    if (exact !== undefined && Math.abs(actual - exact) > 0.5)
      throw new Error(`Native ${axis} ${actual} differs from ${exact}`);
    if (minimum !== undefined && actual < minimum)
      throw new Error(`Native ${axis} ${actual} is below ${minimum}`);
    if (maximum !== undefined && actual > maximum)
      throw new Error(`Native ${axis} ${actual} exceeds ${maximum}`);
  }
}

/** Native play functions operate the actual projected controls and browser input owner. */
export async function runNativeInteractions(
  frame: HTMLIFrameElement,
  interactions: unknown[],
  storyArgs: Record<string, unknown> = {},
): Promise<void> {
  let lastTarget: HTMLElement | null = null;
  const navigationIntents: { url: string; newTab: boolean }[] = [];
  const navigationWindow = windowOf(frame);
  const onNavigation = (event: Event) => {
    const detail = (event as CustomEvent).detail;
    if (detail && typeof detail.url === "string") navigationIntents.push(detail);
  };
  navigationWindow.addEventListener("zoo:navigation-intent", onNavigation);
  try {
    const painted = async () => {
      const win = windowOf(frame);
      await new Promise<void>((resolve) =>
        win.requestAnimationFrame(() => win.requestAnimationFrame(() => resolve())),
      );
    };
    const condition = async <T>(test: () => T): Promise<T> => {
      const win = windowOf(frame);
      return waitFor(test, { container: win.document.body, timeout: 90_000 });
    };
    for (const raw of interactions) {
      if (!raw || typeof raw !== "object" || !("action" in raw) || typeof raw.action !== "string")
        throw new Error("Invalid native interaction descriptor");
      const step = raw as NativeInteraction;
      if (
        ![
          "click",
          "select",
          "type",
          "key",
          "wait",
          "hover",
          "focus",
          "paste",
          "callback",
          "scroll",
          "fixture",
          "pointer",
          "navigation",
          "overflow",
        ].includes(step.action)
      )
        throw new Error(`Unsupported native interaction action: ${step.action}`);
      if (step.index !== undefined && (!Number.isInteger(step.index) || step.index < 0))
        throw new Error("Native interaction index must be a nonnegative integer");
      const win = windowOf(frame);
      const document = win.document;
      if (step.action === "navigation") {
        await condition(() => {
          if (navigationIntents.length !== step.count)
            throw new Error(
              `Native navigation count ${navigationIntents.length} differs from ${step.count}`,
            );
          const last = navigationIntents.at(-1);
          if (step.urlIncludes && !last?.url.includes(step.urlIncludes))
            throw new Error("Native navigation destination differs");
          if (step.newTab !== undefined && last?.newTab !== step.newTab)
            throw new Error("Native new-tab intent differs");
        });
        continue;
      }
      if (step.action === "overflow") {
        if (document.documentElement.scrollWidth > document.documentElement.clientWidth)
          throw new Error("Native document has horizontal overflow");
        continue;
      }
      if (step.action === "fixture") {
        const controls = storyArgs.fixtureControls as Record<string, unknown> | undefined;
        const control = step.name ? controls?.[step.name] : undefined;
        if (typeof control !== "function") throw new Error(`Missing fixture control: ${step.name}`);
        await control(...(step.args ?? []));
        await painted();
        continue;
      }
      if (step.action === "callback") {
        if (!step.name) throw new Error("Native callback assertion requires name");
        const callback = step.name
          .split(".")
          .reduce<unknown>(
            (value, key) =>
              value !== null && (typeof value === "object" || typeof value === "function")
                ? (value as Record<string, unknown>)[key]
                : undefined,
            storyArgs,
          );
        if (typeof callback !== "function") throw new Error(`Missing Story callback: ${step.name}`);
        const revive = (value: unknown): unknown => {
          if (Array.isArray(value)) return value.map(revive);
          if (!value || typeof value !== "object") return value;
          const item = value as Record<string, unknown>;
          if (item.__zooUndefined === true) return undefined;
          return Object.fromEntries(
            Object.entries(item).map(([key, child]) => [key, revive(child)]),
          );
        };
        await condition(async () => {
          if (step.count !== undefined) await expect(callback).toHaveBeenCalledTimes(step.count);
          if (step.args !== undefined)
            await expect(callback).toHaveBeenCalledWith(...step.args.map(revive));
          if (step.count === undefined && step.args === undefined)
            await expect(callback).toHaveBeenCalled();
        });
        continue;
      }
      if (step.action === "wait" && (step.count !== undefined || step.all)) {
        await condition(() => {
          const matches = narrow(document, match(document, step), step.within, step.directWithin);
          if (step.count !== undefined && matches.length !== step.count)
            throw new Error(`Native count ${matches.length} differs from ${step.count}`);
          if (step.all && !matches.length)
            throw new Error("Native all assertion matched no controls");
          for (const current of matches) assertWait(current, step, win);
        });
        continue;
      }
      if (step.action === "key") {
        if (!step.key) throw new Error("Native key action requires key");
        const parts = step.key.split("+");
        const token = parts.pop()!;
        const key = token === "Space" ? " " : token;
        const code = /^[a-z]$/i.test(key)
          ? `Key${key.toUpperCase()}`
          : token === "Space"
            ? "Space"
            : key;
        const keyCode =
          (
            {
              Enter: 13,
              Tab: 9,
              Escape: 27,
              ArrowLeft: 37,
              ArrowUp: 38,
              ArrowRight: 39,
              ArrowDown: 40,
              Home: 36,
              End: 35,
              Backspace: 8,
              Delete: 46,
              Space: 32,
            } as Record<string, number>
          )[token] ?? (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
        if (!keyCode) throw new Error(`Unsupported native key ${step.key}`);
        const mac = /Mac|iPhone|iPad/.test(win.navigator.platform);
        const modifiers = {
          ctrlKey: parts.includes("Control") || (parts.includes("ControlOrMeta") && !mac),
          metaKey: parts.includes("Meta") || (parts.includes("ControlOrMeta") && mac),
          shiftKey: parts.includes("Shift"),
          altKey: parts.includes("Alt"),
        };
        const deepActive = () => {
          let element = document.activeElement;
          while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement;
          return element;
        };
        const connectedTarget =
          lastTarget?.isConnected && lastTarget.ownerDocument === document ? lastTarget : null;
        const active = deepActive();
        const focusedTarget =
          active && active !== document.body && active !== document.documentElement ? active : null;
        const keyboardTarget =
          activeInput(document) ?? focusedTarget ?? connectedTarget ?? document.body;
        const init = {
          key,
          code,
          keyCode,
          which: keyCode,
          bubbles: true,
          cancelable: true,
          composed: true,
          ...modifiers,
        };
        const before = deepActive();
        const event = new win.KeyboardEvent("keydown", init);
        keyboardTarget.dispatchEvent(event);
        keyboardTarget.dispatchEvent(new win.KeyboardEvent("keyup", init));
        if (token === "Tab" && !event.defaultPrevented && before === deepActive()) {
          const focusable = [
            ...new Set(
              roots(document).flatMap((root) => [
                ...root.querySelectorAll<HTMLElement>(
                  "[tabindex],button,input,textarea,select,a[href]",
                ),
              ]),
            ),
          ].filter(
            (element) =>
              element.tabIndex >= 0 &&
              !isComposeBackingInput(element) &&
              enabled(element) &&
              element.getClientRects().length > 0 &&
              element.getAttribute("aria-hidden") !== "true",
          );
          if (!focusable.length) throw new Error("No native control accepts Tab focus");
          const index = focusable.indexOf(
            (focusedTarget as HTMLElement | null) ?? connectedTarget ?? document.body,
          );
          const next = modifiers.shiftKey
            ? index <= 0
              ? focusable.length - 1
              : index - 1
            : (index + 1) % focusable.length;
          focusable[next].focus();
          lastTarget = focusable[next];
        }
        await painted();
        continue;
      }
      if (step.action === "wait" && step.state === "hidden") {
        await condition(() => {
          const matches = narrow(document, match(document, step), step.within, step.directWithin);
          if (
            matches.some(
              (element) =>
                element.getClientRects().length > 0 &&
                win.getComputedStyle(element).visibility !== "hidden",
            )
          )
            throw new Error("Native control is still visible");
        });
        continue;
      }
      const element = await condition(() => {
        const current = target(document, step);
        if (step.action !== "wait" && !enabled(current))
          throw new Error(`Native ${step.action} targets a disabled control`);
        return current;
      });
      if (step.action === "wait") {
        await condition(() => {
          const current = target(document, step);
          assertWait(current, step, win);
        });
        continue;
      }
      if (!enabled(element)) throw new Error(`Native ${step.action} targets a disabled control`);
      lastTarget = element;
      if (step.action === "click") {
        element.focus();
        element.click();
        await painted();
        continue;
      }
      if (step.action === "pointer") {
        const bounds = element.getBoundingClientRect();
        const x = bounds.x + bounds.width / 2,
          y = bounds.y + bounds.height / 2;
        const canvases = roots(document)
          .flatMap((root) => [...root.querySelectorAll("canvas")])
          .filter((canvas) => {
            const box = canvas.getBoundingClientRect();
            return (
              box.width > 0 &&
              box.height > 0 &&
              x >= box.left &&
              x <= box.right &&
              y >= box.top &&
              y <= box.bottom
            );
          });
        if (canvases.length !== 1)
          throw new Error(`Native pointer expected one canvas; found ${canvases.length}`);
        const canvas = canvases[0],
          box = canvas.getBoundingClientRect();
        const init = {
          clientX: x,
          clientY: y,
          ctrlKey: step.ctrlKey ?? false,
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
          button: 0,
          bubbles: true,
          cancelable: true,
          composed: true,
        };
        for (const [type, buttons] of [
          ["pointerdown", 1],
          ["pointerup", 0],
        ] as const) {
          const event = new win.PointerEvent(type, { ...init, buttons });
          Object.defineProperties(event, {
            offsetX: { value: x - box.left },
            offsetY: { value: y - box.top },
          });
          canvas.dispatchEvent(event);
          const mouse = new win.MouseEvent(type === "pointerdown" ? "mousedown" : "mouseup", {
            ...init,
            buttons,
          });
          Object.defineProperties(mouse, {
            offsetX: { value: x - box.left },
            offsetY: { value: y - box.top },
          });
          canvas.dispatchEvent(mouse);
          await painted();
        }
        continue;
      }
      if (step.action === "scroll") {
        if (!Number.isFinite(step.deltaY)) throw new Error("Native scroll requires finite deltaY");
        const bounds = element.getBoundingClientRect();
        const x = bounds.x + bounds.width / 2,
          y = bounds.y + bounds.height / 2;
        const canvases = roots(document)
          .flatMap((root) => [...root.querySelectorAll("canvas")])
          .filter((canvas) => {
            const box = canvas.getBoundingClientRect();
            return (
              box.width > 0 &&
              box.height > 0 &&
              x >= box.left &&
              x <= box.right &&
              y >= box.top &&
              y <= box.bottom
            );
          });
        if (canvases.length !== 1)
          throw new Error(`Native scroll expected one canvas; found ${canvases.length}`);
        canvases[0].dispatchEvent(
          new win.WheelEvent("wheel", {
            clientX: x,
            clientY: y,
            deltaY: step.deltaY,
            deltaMode: 0,
            bubbles: true,
            cancelable: true,
            composed: true,
          }),
        );
        await painted();
        continue;
      }
      if (step.action === "focus") {
        element.focus();
        await painted();
        continue;
      }
      if (step.action === "hover") {
        let bounds = element.getBoundingClientRect();
        if (!bounds.width || !bounds.height) throw new Error("Native hover target has no bounds");
        for (let attempt = 0; attempt < 6; attempt++) {
          const x = bounds.x + bounds.width / 2,
            y = bounds.y + bounds.height / 2;
          const canvases = [
            ...new Set(roots(document).flatMap((root) => [...root.querySelectorAll("canvas")])),
          ].filter((canvas) => {
            const box = canvas.getBoundingClientRect();
            return (
              box.width > 0 &&
              box.height > 0 &&
              x >= box.left &&
              x <= box.right &&
              y >= box.top &&
              y <= box.bottom
            );
          });
          if (canvases.length > 1) throw new Error("Native hover overlaps multiple canvases");
          for (const receiver of [element, ...canvases]) {
            const box = receiver.getBoundingClientRect();
            const init = {
              clientX: x,
              clientY: y,
              bubbles: true,
              composed: true,
              pointerId: 1,
              pointerType: "mouse",
              isPrimary: true,
            };
            for (const type of ["pointerover", "pointerenter", "pointermove"]) {
              const event = new win.PointerEvent(type, init);
              Object.defineProperties(event, {
                offsetX: { value: x - box.left },
                offsetY: { value: y - box.top },
              });
              receiver.dispatchEvent(event);
            }
            for (const type of ["mouseover", "mouseenter", "mousemove"])
              receiver.dispatchEvent(new win.MouseEvent(type, init));
          }
          await painted();
          const next = target(document, step).getBoundingClientRect();
          if (
            attempt > 0 &&
            bounds.x === next.x &&
            bounds.y === next.y &&
            bounds.width === next.width &&
            bounds.height === next.height
          )
            break;
          if (attempt === 5) throw new Error("Native hover layout did not settle");
          bounds = next;
        }
        continue;
      }
      if (step.action === "select") {
        if (step.value === undefined) throw new Error("Native select action requires value");
        element.focus();
        if (element instanceof win.HTMLSelectElement) {
          const option = [...element.options].find(
            (item) => item.value === step.value || item.label === step.value,
          );
          if (!option) throw new Error(`Native option not found: ${step.value}`);
          if (option.disabled) throw new Error(`Native option is disabled: ${step.value}`);
          element.value = option.value;
          element.dispatchEvent(new win.Event("input", { bubbles: true }));
          element.dispatchEvent(new win.Event("change", { bubbles: true }));
        } else {
          element.click();
          const option = await condition(() =>
            target(document, { action: "click", role: "option", name: step.value, exact: true }),
          );
          if (!enabled(option)) throw new Error(`Native option is disabled: ${step.value}`);
          option.click();
        }
        // Selection is an operation, not an assertion that a controlled value changed.
        // Save-pending/error stories may deliberately retain the old value or rename
        // the control after locale changes. Fixtures assert their own expected outcome.
        await painted();
        continue;
      }
      if (step.action === "paste") {
        if (!step.file) throw new Error("Native paste requires an explicit file fixture");
        const bytes = Uint8Array.from(win.atob(step.file.base64), (char) => char.charCodeAt(0));
        const transfer = new win.DataTransfer();
        transfer.items.add(new win.File([bytes], step.file.name, { type: step.file.type }));
        element.dispatchEvent(
          new win.ClipboardEvent("paste", {
            clipboardData: transfer,
            bubbles: true,
            cancelable: true,
            composed: true,
          }),
        );
        await painted();
        continue;
      }
      if (step.action === "type") {
        element.focus();
        element.click();
        const input = await condition(() => {
          const input = activeInput(document);
          if (!input || input.disabled || input.readOnly)
            throw new Error("Native browser input owner is not ready or writable");
          return input;
        });
        const content = step.text ?? step.value;
        if (content === undefined) throw new Error("Native type requires text or value");
        const replace = step.text === undefined;
        if (replace && input.value.length) {
          // Native Compose owns selection as well as text. Run SelectAll through its
          // real keyboard handler before the browser's replacement edit.
          const mac = /Mac|iPhone|iPad/.test(win.navigator.platform);
          const selectAll = {
            key: "a",
            code: "KeyA",
            keyCode: 65,
            which: 65,
            ctrlKey: !mac,
            metaKey: mac,
            bubbles: true,
            cancelable: true,
            composed: true,
          };
          input.dispatchEvent(new win.KeyboardEvent("keydown", selectAll));
          input.dispatchEvent(new win.KeyboardEvent("keyup", selectAll));
          input.setSelectionRange(0, input.value.length);
          await painted();
        }
        const start = replace ? 0 : (input.selectionStart ?? input.value.length);
        const end = replace ? input.value.length : (input.selectionEnd ?? start);
        const next = input.value.slice(0, start) + content + input.value.slice(end);
        const prototype =
          input instanceof win.HTMLTextAreaElement
            ? win.HTMLTextAreaElement.prototype
            : win.HTMLInputElement.prototype;
        if (typeof Object.getOwnPropertyDescriptor(prototype, "value")?.set !== "function")
          throw new Error("Native input has no browser value setter");
        const inputType = content.length ? "insertText" : "deleteContentBackward";
        // Compose's DomInputStrategy consumes beforeinput; input alone changes only
        // the hidden browser owner and never commits into common Kotlin state.
        const beforeInput = new win.InputEvent("beforeinput", {
          data: content,
          inputType,
          bubbles: true,
          cancelable: true,
          composed: true,
        });
        if (input.dispatchEvent(beforeInput)) {
          Reflect.set(prototype, "value", next, input);
          input.setSelectionRange(start + content.length, start + content.length);
          input.dispatchEvent(
            new win.InputEvent("input", {
              data: content,
              inputType,
              bubbles: true,
              composed: true,
            }),
          );
        }
        await condition(() => {
          if (valueOf(target(document, step)) !== next)
            throw new Error("Native text has not committed to the rendered field");
        });
        continue;
      }
      throw new Error("Unsupported native interaction action");
    }
    await painted();
  } finally {
    navigationWindow.removeEventListener("zoo:navigation-intent", onNavigation);
  }
}
