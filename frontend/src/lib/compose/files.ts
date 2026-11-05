/** Browser file/window primitives. Kotlin owns validation, upload orchestration and every visible UI. */
export function createComposeFiles(viewport: HTMLElement) {
  const owner = viewport.ownerDocument;
  const view = owner.defaultView ?? window;
  const files = new Map<string, File>();
  type Bounds = { left: number; top: number; width: number; height: number };
  const targets = new Map<
    string,
    { enabled: boolean; bounds?: Bounds; receive: (raw: string) => boolean }
  >();
  const uploads = new Map<string, AbortController>();
  let focused: string | undefined;
  let cancelled = false;
  let pickerOwner: string | undefined;
  let finishPicker: ((value: string) => void) | undefined;
  const input = owner.createElement("input");
  input.type = "file";
  input.hidden = true;
  viewport.append(input);
  const describe = (values: File[], origin: string) =>
    JSON.stringify({
      origin,
      files: values.map((file) => {
        const id = crypto.randomUUID();
        files.set(id, file);
        return { id, name: file.name, type: file.type, size: file.size };
      }),
    });
  const onPick = () => {
    finishPicker?.(describe(Array.from(input.files ?? []), "picker"));
    finishPicker = undefined;
    pickerOwner = undefined;
    input.value = "";
  };
  input.addEventListener("change", onPick);
  input.addEventListener("cancel", onPick);
  const clipboard = (data: DataTransfer | null) => [
    ...Array.from(data?.files ?? []),
    ...Array.from(data?.items ?? []).flatMap((item) =>
      item.kind === "file" ? [item.getAsFile()].filter((file): file is File => file !== null) : [],
    ),
  ];
  const paste = (event: ClipboardEvent) => {
    if (!event.composedPath().includes(viewport)) return;
    let target = focused ? targets.get(focused) : undefined;
    // A paste explicitly dispatched to an unfocused logical textbox has an owner
    // too. Resolve its Kotlin-registered bounds without creating or focusing an
    // invisible browser editor. Real focused-input paste retains its usual route.
    if (!focused) {
      const element = event.target as HTMLElement | null;
      if (!element || !["textbox", "searchbox"].includes(element.getAttribute?.("role") ?? ""))
        return;
      const rect = element.getBoundingClientRect();
      if (!rect.width || !rect.height) return;
      const x = rect.left + rect.width / 2,
        y = rect.top + rect.height / 2;
      const candidates = [...targets.values()].filter(
        ({ enabled, bounds }) =>
          enabled &&
          bounds &&
          x >= bounds.left &&
          x <= bounds.left + bounds.width &&
          y >= bounds.top &&
          y <= bounds.top + bounds.height,
      );
      if (candidates.length !== 1) return;
      target = candidates[0];
    }
    if (!target?.enabled) return;
    const values = clipboard(event.clipboardData);
    if (values.length && target.receive(describe(values, "paste"))) event.preventDefault();
  };
  owner.addEventListener("paste", paste, true);
  const base64 = (bytes: Uint8Array) => {
    let result = "";
    for (let offset = 0; offset < bytes.length; offset += 32768)
      result += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
    return btoa(result);
  };
  const safeUrl = (raw: string) => {
    const url = new URL(raw);
    if (!["https:", "http:"].includes(url.protocol) || url.username || url.password)
      throw new Error("Unsupported storage URL");
    return url;
  };
  return {
    target(id: string, raw: string, receive: (raw: string) => boolean) {
      const state = JSON.parse(raw);
      targets.set(id, {
        enabled: state.enabled,
        bounds: state.bounds,
        receive,
      });
    },
    forget(id: string) {
      targets.delete(id);
      if (focused === id) focused = undefined;
      if (pickerOwner === id) {
        finishPicker?.("{}");
        finishPicker = undefined;
        pickerOwner = undefined;
      }
    },
    focus(id: string) {
      focused = id || undefined;
    },
    release(id: string) {
      uploads.get(id)?.abort();
      files.delete(id);
    },
    async invoke(name: string, payload: string): Promise<string> {
      if (cancelled) throw new Error("Compose frontend was disposed");
      if (name === "pickFiles") {
        const options = JSON.parse(payload);
        pickerOwner = options.owner;
        if (!input.isConnected) viewport.append(input);
        input.accept = options.accept;
        input.multiple = options.multiple;
        finishPicker?.("{}");
        return new Promise((resolve) => {
          finishPicker = resolve;
          input.click();
        });
      }
      if (name === "fileRead") {
        const file = files.get(payload);
        if (!file) throw new Error("File was released");
        return base64(new Uint8Array(await file.arrayBuffer()));
      }
      if (name === "filePut") {
        const { id, url, type } = JSON.parse(payload);
        const file = files.get(id);
        if (!file) throw new Error("File was released");
        const controller = new AbortController();
        uploads.set(id, controller);
        try {
          const response = await fetch(safeUrl(url), {
            method: "PUT",
            body: file,
            headers: { "Content-Type": type },
            credentials: "omit",
            referrerPolicy: "no-referrer",
            signal: controller.signal,
          });
          if (!response.ok) throw new Error(`Upload failed: ${response.status}`);
          return "{}";
        } finally {
          uploads.delete(id);
        }
      }
      if (name === "attachmentDownload") {
        const { url, name: fileName } = JSON.parse(payload);
        const controller = new AbortController();
        const key = crypto.randomUUID();
        uploads.set(key, controller);
        try {
          const response = await fetch(safeUrl(url), {
            credentials: "omit",
            referrerPolicy: "no-referrer",
            signal: controller.signal,
          });
          if (!response.ok) throw new Error(`Download failed: ${response.status}`);
          const objectUrl = URL.createObjectURL(await response.blob());
          try {
            const anchor = owner.createElement("a");
            anchor.href = objectUrl;
            anchor.download = fileName;
            anchor.rel = "noopener";
            owner.body.append(anchor);
            anchor.click();
            anchor.remove();
          } finally {
            view.setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
          }
        } finally {
          uploads.delete(key);
        }
        return "{}";
      }
      throw new Error("Unknown file primitive");
    },
    close() {
      cancelled = true;
      finishPicker?.("{}");
      uploads.forEach((controller) => controller.abort());
      files.clear();
      targets.clear();
      uploads.clear();
      input.remove();
      owner.removeEventListener("paste", paste, true);
    },
  };
}
