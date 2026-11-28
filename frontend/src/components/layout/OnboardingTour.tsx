import { readStorageValue, writeStorageValue } from "../../lib/storage";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { CompassIcon } from "lucide-react";
import { Button } from "../ui/button";

const TourContext = createContext<(() => void) | null>(null);
const steps = ["welcome", "channels", "messages", "notifications", "slack", "settings"] as const;
export type OnboardingStep = (typeof steps)[number];

/** Highlights the real screen without intercepting mouse, touch or keyboard input. */
function useTourTarget(step: OnboardingStep, active: boolean) {
  useEffect(() => {
    if (!active || step === "welcome") return;
    let target: HTMLElement | null = null;
    const update = () => {
      const next =
        Array.from(document.querySelectorAll<HTMLElement>(`[data-onboarding="${step}"]`)).find(
          (element) => element.getClientRects().length > 0,
        ) ?? null;
      if (next === target) return;
      target?.classList.remove("onboarding-target");
      target = next;
      target?.classList.add("onboarding-target");
      target?.scrollIntoView({ block: "nearest", inline: "nearest" });
    };
    const observer = new MutationObserver(update);
    observer.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("resize", update);
    update();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
      target?.classList.remove("onboarding-target");
    };
  }, [step, active]);
}

export function OnboardingProvider({
  userId,
  children,
  onStepChange,
  channelSelected = true,
}: {
  userId: string | null;
  children: ReactNode;
  onStepChange?: (step: OnboardingStep) => void | Promise<void>;
  channelSelected?: boolean;
}) {
  const { t } = useTranslation();
  const storageKey = `zoo:onboarding:v1:${userId}`;
  const [open, setOpen] = useState(() => !!userId && readStorageValue(storageKey) !== "done");
  const [index, setIndex] = useState(0);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);
  const panel = useRef<HTMLElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const trigger = useRef<HTMLElement | null>(null);
  const generation = useRef(0);
  const step = steps[index];
  useTourTarget(step, open);
  const close = useCallback(() => {
    generation.current += 1;
    setPending(false);
    setOpen(false);
    if (userId) writeStorageValue(storageKey, "done");
    if (trigger.current?.isConnected) trigger.current.focus();
  }, [storageKey, userId]);

  useEffect(() => {
    if (!open || !panel.current) return;
    document.documentElement.dataset.onboardingActive = "true";
    const resize = new ResizeObserver(() => {
      document.documentElement.style.setProperty(
        "--onboarding-panel-height",
        `${panel.current!.getBoundingClientRect().height + 24}px`,
      );
    });
    resize.observe(panel.current);
    return () => {
      resize.disconnect();
      delete document.documentElement.dataset.onboardingActive;
      document.documentElement.style.removeProperty("--onboarding-panel-height");
    };
  }, [open]);
  useEffect(() => {
    if (open) heading.current?.focus({ preventScroll: true });
  }, [open, index]);
  useEffect(() => {
    if (!open) return;
    const escape = (event: KeyboardEvent) => {
      if (
        event.key === "Escape" &&
        !event.isComposing &&
        event.keyCode !== 229 &&
        !event.defaultPrevented &&
        !document.querySelector('[role="dialog"][aria-modal="true"]')
      )
        close();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [open, close]);

  const move = async (next: number) => {
    if (pending) return;
    const started = generation.current;
    setPending(true);
    setError(false);
    try {
      await onStepChange?.(steps[next]);
      if (started === generation.current) setIndex(next);
    } catch {
      if (started === generation.current) setError(true);
    } finally {
      if (started === generation.current) setPending(false);
    }
  };
  return (
    <TourContext.Provider
      value={() => {
        generation.current += 1;
        setPending(false);
        trigger.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null;
        setIndex(0);
        setError(false);
        setOpen(true);
      }}
    >
      {children}
      {open && (
        <aside
          ref={panel}
          data-onboarding-panel
          aria-label={t("onboarding.title")}
          className="fixed inset-x-3 bottom-3 z-40 flex max-h-[45dvh] flex-col gap-3 overflow-y-auto rounded-xl border bg-popover p-4 text-popover-foreground shadow-lg md:top-3 md:right-3 md:bottom-auto md:left-auto md:max-h-[calc(100dvh-1.5rem)] md:w-80"
        >
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <CompassIcon aria-hidden="true" className="size-4" />
            <span>{t("onboarding.title")}</span>
            <span className="ml-auto">
              {t("onboarding.progress", { current: index + 1, total: steps.length })}
            </span>
          </div>
          <div aria-live="polite" aria-atomic="true">
            <h2
              ref={heading}
              tabIndex={-1}
              className="mb-2 font-heading text-base font-semibold outline-none"
            >
              {t(`onboarding.${step}.title`)}
            </h2>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {t(`onboarding.${step}.description`)}
            </p>
          </div>
          {step === "channels" && !channelSelected && (
            <p className="text-xs text-muted-foreground">{t("onboarding.chooseChannel")}</p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {t("onboarding.navigationError")}
            </p>
          )}
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={close}>
              {t("onboarding.skip")}
            </Button>
            {index > 0 && (
              <Button
                variant="outline"
                size="sm"
                disabled={pending}
                onClick={() => void move(index - 1)}
              >
                {t("onboarding.back")}
              </Button>
            )}
            <Button
              size="sm"
              disabled={pending || (step === "channels" && !channelSelected)}
              onClick={() => (index === steps.length - 1 ? close() : void move(index + 1))}
            >
              {t(index === steps.length - 1 ? "onboarding.finish" : "onboarding.next")}
            </Button>
          </div>
        </aside>
      )}
    </TourContext.Provider>
  );
}

export function OnboardingButton() {
  const { t } = useTranslation();
  const start = useContext(TourContext);
  if (!start) return null;
  return (
    <Button
      variant="ghost"
      size="icon"
      className="size-10"
      onClick={start}
      aria-label={t("onboarding.replay")}
      title={t("onboarding.replay")}
    >
      <CompassIcon aria-hidden="true" className="size-5 text-muted-foreground" />
    </Button>
  );
}
