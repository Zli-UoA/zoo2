/** Browser file/window primitives. Kotlin owns validation, upload orchestration and every visible UI. */
export function createComposeFiles(viewport: HTMLElement) {
  const files = new Map<string, File>();
  const targets = new Map<string, { enabled: boolean; receive: (raw: string) => boolean }>();
  const uploads = new Map<string, AbortController>();
  const popups = new Map<string, Window>();
  let focused: string | undefined;
  let cancelled = false;
  let pickerOwner: string | undefined;
  let finishPicker: ((value: string) => void) | undefined;
  const input = document.createElement("input");
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
    const target = focused ? targets.get(focused) : undefined;
    if (!target?.enabled || !event.composedPath().includes(viewport)) return;
    const values = clipboard(event.clipboardData);
    if (values.length && target.receive(describe(values, "paste"))) event.preventDefault();
  };
  document.addEventListener("paste", paste, true);
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
    reserve(id: string) {
      const popup = window.open("about:blank", "_blank");
      if (popup) {
        popup.opener = null;
        popups.set(id, popup);
      }
    },
    async invoke(name: string, payload: string): Promise<string> {
      if (cancelled) throw new Error("Compose frontend was disposed");
      if (name === "pickFiles") {
        const options = JSON.parse(payload);
        pickerOwner = options.owner;
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
      if (name === "attachmentOpen") {
        const { id, url } = JSON.parse(payload);
        const popup = popups.get(id);
        popups.delete(id);
        if (popup) {
          if (url) popup.location.replace(safeUrl(url).href);
          else popup.close();
        }
        return "{}";
      }
      throw new Error("Unknown file primitive");
    },
    close() {
      cancelled = true;
      finishPicker?.("{}");
      uploads.forEach((controller) => controller.abort());
      popups.forEach((popup) => popup.close());
      files.clear();
      targets.clear();
      uploads.clear();
      popups.clear();
      input.remove();
      document.removeEventListener("paste", paste, true);
    },
  };
}
