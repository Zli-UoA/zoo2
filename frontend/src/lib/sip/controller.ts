import type { ApiClient } from "../api/client";
import type { PhoneConnection, SipState } from "./types";

export interface SipDriver {
  connect(this: void): Promise<void>;
  disconnect(this: void): Promise<void>;
  call(this: void, target: string): Promise<void>;
  answer(this: void): Promise<void>;
  hangup(this: void): Promise<void>;
  mute(this: void, muted: boolean): void;
  dtmf(this: void, tone: string): Promise<void>;
}
export interface SipEvents {
  ready(this: void): void;
  incoming(this: void): void;
  answered(this: void): void;
  ended(this: void): void;
  disconnected(this: void): void;
}
export type SipDriverFactory = (
  credentials: PhoneConnection,
  events: SipEvents,
) => Promise<SipDriver>;
export const initialSipState: SipState = {
  status: "offline",
  lineId: null,
  muted: false,
  busy: false,
  error: false,
  peer: "",
};
export function dialTarget(number: string): string {
  const target = number.trim();
  if (!/^\+?\d{1,32}$/.test(target)) throw new Error("Invalid telephone number");
  return target;
}

/** One transport/registration for the selected line, shared by the entire shell.
 * The gateway session is owned by the authenticated user. Generations dispose late connects after logout.
 */
export class SipController {
  private state: SipState = initialSipState;
  private listeners = new Set<() => void>();
  private driver: SipDriver | null = null;
  private generation = 0;
  private abort: AbortController | null = null;
  private registrationTimeout: ReturnType<typeof setTimeout> | undefined;
  private revision = "";
  constructor(
    private api: ApiClient,
    private factory: SipDriverFactory,
  ) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.state;
  private set(patch: Partial<SipState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  private clearTimeout() {
    clearTimeout(this.registrationTimeout);
    this.registrationTimeout = undefined;
  }
  disconnect = () => {
    this.generation++;
    this.clearTimeout();
    this.abort?.abort();
    this.abort = null;
    const driver = this.driver;
    this.driver = null;
    this.set({ ...initialSipState });
    if (driver) void driver.disconnect().catch(() => {});
  };
  connect = async (lineId: string) => {
    this.disconnect();
    const generation = this.generation;
    const abort = new AbortController();
    this.abort = abort;
    this.set({ status: "connecting", lineId, busy: true });
    this.registrationTimeout = setTimeout(() => {
      if (generation === this.generation) {
        this.disconnect();
        this.set({ error: true });
      }
    }, 15000);
    try {
      const { data } = await this.api.post<PhoneConnection>(
        `/api/sip/lines/${encodeURIComponent(lineId)}/connect`,
        undefined,
        { signal: abort.signal },
      );
      if (generation !== this.generation) return;
      const current = () => generation === this.generation;
      const driver = await this.factory(data, {
        ready: () => {
          if (current()) {
            this.clearTimeout();
            this.set({ status: "ready", busy: false, error: false });
          }
        },
        incoming: () => {
          if (current()) this.set({ status: "incoming", peer: "", muted: false, busy: false });
        },
        answered: () => {
          if (current()) this.set({ status: "active", busy: false });
        },
        ended: () => {
          if (current()) this.set({ status: "ready", peer: "", muted: false, busy: false });
        },
        disconnected: () => {
          if (current()) {
            this.disconnect();
            this.set({ error: true });
          }
        },
      });
      if (!current()) {
        void driver.disconnect().catch(() => {});
        return;
      }
      this.driver = driver;
      this.revision = data.revision;
      await driver.connect();
    } catch {
      if (generation === this.generation) {
        this.disconnect();
        this.set({ error: true });
      }
    }
  };
  private async action(run: (driver: SipDriver) => Promise<void>) {
    if (!this.driver || this.state.busy) return;
    const generation = this.generation;
    this.set({ busy: true, error: false });
    try {
      await run(this.driver);
    } catch {
      if (generation === this.generation) this.set({ error: true });
    } finally {
      if (generation === this.generation) this.set({ busy: false });
    }
  }
  private async authorize() {
    if (!this.state.lineId) throw new Error("Not connected");
    // Recheck the displayed line revision; the server also authorizes every gateway command.
    const generation = this.generation;
    try {
      const { data } = await this.api.post<{ revision: string }>(
        `/api/sip/lines/${encodeURIComponent(this.state.lineId)}/authorize`,
        undefined,
        { signal: this.abort?.signal },
      );
      if (generation !== this.generation) return;
      if (data.revision !== this.revision) throw new Error("Connection settings changed");
    } catch (error) {
      if (generation === this.generation) {
        this.disconnect();
        this.set({ error: true });
      }
      throw error;
    }
  }

  dial = async (number: string) => {
    if (this.state.status !== "ready") return;
    let target: string;
    try {
      target = dialTarget(number);
    } catch {
      this.set({ error: true });
      return;
    }
    await this.action(async (driver) => {
      await this.authorize();
      if (driver !== this.driver) return;
      this.set({ status: "dialing", peer: number.trim(), muted: false });
      try {
        await driver.call(target);
      } catch (error) {
        if (driver === this.driver) this.set({ status: "ready", peer: "" });
        throw error;
      }
    });
  };
  answer = () =>
    this.action(async (driver) => {
      if (this.state.status !== "incoming") return;
      await this.authorize();
      if (driver === this.driver) await driver.answer();
    });
  hangup = () => this.action((driver) => driver.hangup());
  dtmf = (tone: string) =>
    this.action(async (driver) => {
      if (this.state.status === "active" && /^[0-9*#]$/.test(tone)) await driver.dtmf(tone);
    });
  toggleMute = () => {
    if (!this.driver || this.state.status !== "active") return;
    const muted = !this.state.muted;
    this.driver.mute(muted);
    this.set({ muted });
  };
}
