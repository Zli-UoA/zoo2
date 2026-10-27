// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { CallBar } from "../../components/chat/CallBar";
import { LocaleProvider } from "../../components/i18n/LocaleProvider";
import { resolveConfig } from "../config";
import { CallProvider, useCall } from "./call-context";

const testConfig = resolveConfig({ DEV: true, VITE_DEV_USER_ID: "dev-user-1" });

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("CallContext", () => {
  it("throws error when used outside CallProvider", () => {
    function Consumer() {
      useCall();
      return null;
    }

    expect(() => {
      act(() => {
        root.render(createElement(Consumer));
      });
    }).toThrow("useCall must be used within a CallProvider");
  });

  it("provides default idle state within CallProvider", () => {
    let capturedState: ReturnType<typeof useCall> | null = null;

    function Consumer() {
      capturedState = useCall();
      return createElement("div", { id: "rendered" });
    }

    act(() => {
      root.render(
        createElement(
          LocaleProvider,
          null,
          createElement(CallProvider, { config: testConfig }, createElement(Consumer)),
        ),
      );
    });

    expect(capturedState).not.toBeNull();
    const state = capturedState!;
    expect(state.currentChannelId).toBeNull();
    expect(state.connectionState).toBe("idle");
    expect(state.isMuted).toBe(false);
    expect(state.participants).toEqual([]);
    expect(document.getElementById("rendered")).not.toBeNull();
  });

  it("CallBar renders nothing when connectionState is idle", () => {
    act(() => {
      root.render(
        createElement(
          LocaleProvider,
          null,
          createElement(CallProvider, { config: testConfig }, createElement(CallBar)),
        ),
      );
    });

    expect(document.querySelector("aside")).toBeNull();
  });
});
